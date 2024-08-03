#[cfg(feature = "a11y")]
pub mod adapter;
pub mod control_flow;
pub mod proxy;
pub mod state;

#[cfg(feature = "a11y")]
use crate::application::SurfaceIdWrapper;
use crate::{
    application::Event,
    conversion,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition},
    handlers::{
        activation::IcedRequestData,
        wp_fractional_scaling::FractionalScalingManager,
        wp_viewporter::ViewporterState,
    },
    sctk_event::{
        DataSourceEvent, DndOfferEvent, IcedSctkEvent,
        LayerSurfaceEventVariant, PopupEventVariant, SctkEvent, StartCause,
        WindowEventVariant,
    },
    settings,
    subsurface_widget::SubsurfaceState,
};
use iced_futures::core::window::Mode;
use iced_runtime::command::platform_specific::{
    self,
    wayland::{
        data_device::DndIcon, layer_surface::SctkLayerSurfaceSettings,
        window::SctkWindowSettings,
    },
};
use sctk::{
    activation::{ActivationState, RequestData},
    compositor::CompositorState,
    data_device_manager::DataDeviceManagerState,
    globals::GlobalData,
    output::OutputState,
    reexports::{
        calloop::{self, EventLoop, PostAction},
        client::{
            globals::registry_queue_init, protocol::wl_surface::WlSurface,
            ConnectError, Connection, DispatchError, Proxy,
        },
    },
    registry::RegistryState,
    seat::SeatState,
    session_lock::SessionLockState,
    shell::{
        wlr_layer::{LayerShell, LayerSurface},
        xdg::XdgShell,
        WaylandSurface,
    },
    shm::Shm,
};
use sctk::{
    data_device_manager::data_source::DragSource,
    reexports::calloop_wayland_source::WaylandSource,
};
#[cfg(feature = "a11y")]
use std::sync::{Arc, Mutex};
use std::{
    collections::HashMap,
    fmt::Debug,
    io::{BufRead, BufReader},
    num::NonZeroU32,
    time::{Duration, Instant},
};
use tracing::error;
use wayland_backend::client::WaylandError;

use self::{
    control_flow::ControlFlow,
    state::{Dnd, LayerSurfaceCreationError, SctkState},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Features {
    // TODO
}

pub struct SctkEventLoop<T> {
    // TODO after merged
    // pub data_device_manager_state: DataDeviceManagerState,
    pub(crate) event_loop: EventLoop<'static, SctkState<T>>,
    pub(crate) wayland_dispatcher:
        calloop::Dispatcher<'static, WaylandSource<SctkState<T>>, SctkState<T>>,
    pub(crate) _features: Features,
    /// A proxy to wake up event loop.
    pub event_loop_awakener: calloop::ping::Ping,
    /// A sender for submitting user events in the event loop
    pub user_events_sender: calloop::channel::Sender<Event<T>>,
    pub(crate) state: SctkState<T>,

    #[cfg(feature = "a11y")]
    pub(crate) a11y_events: Arc<Mutex<Vec<adapter::A11yWrapper>>>,
}

impl<T> SctkEventLoop<T>
where
    T: 'static + Debug,
{
    pub(crate) fn new<F: Sized>(
        _settings: &settings::Settings<F>,
    ) -> Result<Self, ConnectError> {
        let connection = Connection::connect_to_env()?;
        let _display = connection.display();
        let (globals, event_queue) = registry_queue_init(&connection).unwrap();
        let event_loop = calloop::EventLoop::<SctkState<T>>::try_new().unwrap();
        let loop_handle = event_loop.handle();

        let qh = event_queue.handle();
        let registry_state = RegistryState::new(&globals);

        let (ping, ping_source) = calloop::ping::make_ping().unwrap();
        // TODO
        loop_handle
            .insert_source(ping_source, |_, _, _state| {
                // Drain events here as well to account for application doing batch event processing
                // on RedrawEventsCleared.
                // shim::handle_window_requests(state);
            })
            .unwrap();
        let (user_events_sender, user_events_channel) =
            calloop::channel::channel();

        loop_handle
            .insert_source(user_events_channel, |event, _, state| match event {
                calloop::channel::Event::Msg(e) => {
                    state.pending_user_events.push(e);
                }
                calloop::channel::Event::Closed => {}
            })
            .unwrap();
        let wayland_source =
            WaylandSource::new(connection.clone(), event_queue);

        let wayland_dispatcher = calloop::Dispatcher::new(
            wayland_source,
            |_, queue, winit_state| queue.dispatch_pending(winit_state),
        );

        let _wayland_source_dispatcher = event_loop
            .handle()
            .register_dispatcher(wayland_dispatcher.clone())
            .unwrap();

        let (viewporter_state, fractional_scaling_manager) =
            match FractionalScalingManager::new(&globals, &qh) {
                Ok(m) => {
                    let viewporter_state =
                        match ViewporterState::new(&globals, &qh) {
                            Ok(s) => Some(s),
                            Err(e) => {
                                error!(
                                    "Failed to initialize viewporter: {}",
                                    e
                                );
                                None
                            }
                        };
                    (viewporter_state, Some(m))
                }
                Err(e) => {
                    error!(
                        "Failed to initialize fractional scaling manager: {}",
                        e
                    );
                    (None, None)
                }
            };

        Ok(Self {
            event_loop,
            wayland_dispatcher,
            state: SctkState {
                connection,
                registry_state,
                seat_state: SeatState::new(&globals, &qh),
                output_state: OutputState::new(&globals, &qh),
                compositor_state: CompositorState::bind(&globals, &qh)
                    .expect("wl_compositor is not available"),
                shm_state: Shm::bind(&globals, &qh)
                    .expect("wl_shm is not available"),
                xdg_shell_state: XdgShell::bind(&globals, &qh)
                    .expect("xdg shell is not available"),
                layer_shell: LayerShell::bind(&globals, &qh).ok(),
                data_device_manager_state: DataDeviceManagerState::bind(
                    &globals, &qh,
                )
                .expect("data device manager is not available"),
                activation_state: ActivationState::bind(&globals, &qh).ok(),
                session_lock_state: SessionLockState::new(&globals, &qh),
                session_lock: None,

                queue_handle: qh,
                loop_handle,

                _cursor_surface: None,
                _multipool: None,
                outputs: Vec::new(),
                seats: Vec::new(),
                windows: Vec::new(),
                layer_surfaces: Vec::new(),
                popups: Vec::new(),
                lock_surfaces: Vec::new(),
                dnd_source: None,
                _kbd_focus: None,
                touch_points: HashMap::new(),
                sctk_events: Vec::new(),
                frame_events: Vec::new(),
                pending_user_events: Vec::new(),
                token_ctr: 0,
                _accept_counter: 0,
                dnd_offer: None,
                fractional_scaling_manager,
                viewporter_state,
                compositor_updates: Default::default(),
            },
            _features: Default::default(),
            event_loop_awakener: ping,
            user_events_sender,
            #[cfg(feature = "a11y")]
            a11y_events: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn proxy(&self) -> proxy::Proxy<Event<T>> {
        proxy::Proxy::new(self.user_events_sender.clone())
    }

    pub fn get_layer_surface(
        &mut self,
        layer_surface: SctkLayerSurfaceSettings,
    ) -> Result<(iced_runtime::window::Id, WlSurface), LayerSurfaceCreationError>
    {
        self.state.get_layer_surface(layer_surface)
    }

    pub fn get_window(
        &mut self,
        settings: SctkWindowSettings,
    ) -> (iced_runtime::window::Id, WlSurface) {
        self.state.get_window(settings)
    }

    // TODO Ashley provide users a reasonable method of setting the role for the surface
    #[cfg(feature = "a11y")]
    pub fn init_a11y_adapter(
        &mut self,
        surface: &WlSurface,
        app_id: Option<String>,
        surface_title: Option<String>,
        _role: iced_accessibility::accesskit::Role,
    ) -> adapter::IcedSctkAdapter {
        use iced_accessibility::{
            accesskit::{
                NodeBuilder, NodeClassSet, NodeId, Role, Tree, TreeUpdate,
            },
            accesskit_unix::Adapter,
            window_node_id,
        };
        let node_id = window_node_id();
        let event_list = self.a11y_events.clone();
        adapter::IcedSctkAdapter {
            adapter: Adapter::new(
                move || {
                    event_list
                        .lock()
                        .unwrap()
                        .push(adapter::A11yWrapper::Enabled);
                    let mut node = NodeBuilder::new(Role::Window);
                    if let Some(name) = surface_title {
                        node.set_name(name);
                    }
                    let node = node.build(&mut NodeClassSet::lock_global());
                    let root = NodeId(node_id);
                    TreeUpdate {
                        nodes: vec![(root, node)],
                        tree: Some(Tree::new(root)),
                        focus: root,
                    }
                },
                Box::new(adapter::IcedSctkActionHandler {
                    wl_surface: surface.clone(),
                    event_list: self.a11y_events.clone(),
                }),
            ),
            id: node_id,
        }
    }

    pub fn run_return<F>(&mut self, mut callback: F) -> i32
    where
        F: FnMut(IcedSctkEvent<T>, &SctkState<T>, &mut ControlFlow),
    {
        let mut control_flow = ControlFlow::Poll;

        let mut cursor_position = HashMap::<_, LogicalPosition<f64>>::new();

        callback(
            IcedSctkEvent::NewEvents(StartCause::Init),
            &self.state,
            &mut control_flow,
        );

        // XXX don't re-bind?
        let wl_compositor = self
            .state
            .registry_state
            .bind_one(&self.state.queue_handle, 1..=6, GlobalData)
            .unwrap();
        let wl_subcompositor = self.state.registry_state.bind_one(
            &self.state.queue_handle,
            1..=1,
            GlobalData,
        );
        let wp_viewporter = self.state.registry_state.bind_one(
            &self.state.queue_handle,
            1..=1,
            GlobalData,
        );
        let wl_shm = self
            .state
            .registry_state
            .bind_one(&self.state.queue_handle, 1..=1, GlobalData)
            .unwrap();
        let wp_dmabuf = self
            .state
            .registry_state
            .bind_one(&self.state.queue_handle, 2..=4, GlobalData)
            .ok();
        let wp_alpha_modifier = self
            .state
            .registry_state
            .bind_one(&self.state.queue_handle, 1..=1, ())
            .ok();
        if let Ok(wl_subcompositor) = wl_subcompositor {
            if let Ok(wp_viewporter) = wp_viewporter {
                callback(
                    IcedSctkEvent::Subcompositor(SubsurfaceState {
                        wl_compositor,
                        wl_subcompositor,
                        wp_viewporter,
                        wl_shm,
                        wp_dmabuf,
                        wp_alpha_modifier,
                        qh: self.state.queue_handle.clone(),
                        buffers: HashMap::new(),
                        unmapped_subsurfaces: Vec::new(),
                    }),
                    &self.state,
                    &mut control_flow,
                );
            } else {
                tracing::warn!(
                    "No `wp_viewporter`. Subsurfaces not supported."
                );
            }
        } else {
            tracing::warn!("No `wl_subcompositor`. Subsurfaces not supported.");
        }

        let mut sctk_event_sink_back_buffer = Vec::new();
        let mut compositor_event_back_buffer = Vec::new();
        let mut frame_event_back_buffer = Vec::new();

        // NOTE We break on errors from dispatches, since if we've got protocol error
        // libwayland-client/wayland-rs will inform us anyway, but crashing downstream is not
        // really an option. Instead we inform that the event loop got destroyed. We may
        // communicate an error that something was terminated, but winit doesn't provide us
        // with an API to do that via some event.
        // Still, we set the exit code to the error's OS error code, or to 1 if not possible.
        let exit_code = loop {
            // Send pending events to the server.
            match self.wayland_dispatcher.as_source_ref().connection().flush() {
                Ok(_) => {}
                Err(error) => {
                    break match error {
                        WaylandError::Io(err) => err.raw_os_error(),
                        WaylandError::Protocol(_) => None,
                    }
                    .unwrap_or(1)
                }
            }

            // During the run of the user callback, some other code monitoring and reading the
            // Wayland socket may have been run (mesa for example does this with vsync), if that
            // is the case, some events may have been enqueued in our event queue.
            //
            // If some messages are there, the event loop needs to behave as if it was instantly
            // woken up by messages arriving from the Wayland socket, to avoid delaying the
            // dispatch of these events until we're woken up again.
            let instant_wakeup = {
                let mut wayland_source =
                    self.wayland_dispatcher.as_source_mut();
                let queue = wayland_source.queue();
                match queue.dispatch_pending(&mut self.state) {
                    Ok(dispatched) => dispatched > 0,
                    // TODO better error handling
                    Err(error) => {
                        break match error {
                            DispatchError::BadMessage { .. } => None,
                            DispatchError::Backend(err) => match err {
                                WaylandError::Io(err) => err.raw_os_error(),
                                WaylandError::Protocol(_) => None,
                            },
                        }
                        .unwrap_or(1)
                    }
                }
            };

            match control_flow {
                ControlFlow::ExitWithCode(code) => break code,
                ControlFlow::Poll => {
                    // Non-blocking dispatch.
                    let timeout = Duration::from_millis(0);
                    if let Err(error) =
                        self.event_loop.dispatch(Some(timeout), &mut self.state)
                    {
                        break raw_os_err(error);
                    }

                    callback(
                        IcedSctkEvent::NewEvents(StartCause::Poll),
                        &self.state,
                        &mut control_flow,
                    );
                }
                ControlFlow::Wait => {
                    let timeout = if instant_wakeup {
                        Some(Duration::from_millis(0))
                    } else {
                        None
                    };

                    if let Err(error) =
                        self.event_loop.dispatch(timeout, &mut self.state)
                    {
                        break raw_os_err(error);
                    }

                    callback(
                        IcedSctkEvent::NewEvents(StartCause::WaitCancelled {
                            start: Instant::now(),
                            requested_resume: None,
                        }),
                        &self.state,
                        &mut control_flow,
                    );
                }
                ControlFlow::WaitUntil(deadline) => {
                    let start = Instant::now();

                    // Compute the amount of time we'll block for.
                    let duration = if deadline > start && !instant_wakeup {
                        deadline - start
                    } else {
                        Duration::from_millis(0)
                    };

                    if let Err(error) = self
                        .event_loop
                        .dispatch(Some(duration), &mut self.state)
                    {
                        break raw_os_err(error);
                    }

                    let now = Instant::now();

                    if now < deadline {
                        callback(
                            IcedSctkEvent::NewEvents(
                                StartCause::WaitCancelled {
                                    start,
                                    requested_resume: Some(deadline),
                                },
                            ),
                            &self.state,
                            &mut control_flow,
                        )
                    } else {
                        callback(
                            IcedSctkEvent::NewEvents(
                                StartCause::ResumeTimeReached {
                                    start,
                                    requested_resume: deadline,
                                },
                            ),
                            &self.state,
                            &mut control_flow,
                        )
                    }
                }
            }

            // handle compositor events
            std::mem::swap(
                &mut compositor_event_back_buffer,
                &mut self.state.compositor_updates,
            );

            for event in compositor_event_back_buffer.drain(..) {
                let forward_event = match &event {
                    SctkEvent::LayerSurfaceEvent {
                        variant:
                            LayerSurfaceEventVariant::ScaleFactorChanged(..),
                        ..
                    }
                    | SctkEvent::PopupEvent {
                        variant: PopupEventVariant::ScaleFactorChanged(..),
                        ..
                    }
                    | SctkEvent::WindowEvent {
                        variant: WindowEventVariant::ScaleFactorChanged(..),
                        ..
                    } => true,
                    // ignore other events that shouldn't be in this buffer
                    event => {
                        tracing::warn!(
                            "Unhandled compositor event: {:?}",
                            event
                        );
                        false
                    }
                };
                if forward_event {
                    sticky_exit_callback(
                        IcedSctkEvent::SctkEvent(event),
                        &self.state,
                        &mut control_flow,
                        &mut callback,
                    );
                }
            }

            std::mem::swap(
                &mut frame_event_back_buffer,
                &mut self.state.frame_events,
            );

            for event in frame_event_back_buffer.drain(..) {
                sticky_exit_callback(
                    IcedSctkEvent::Frame(event.0, event.1),
                    &self.state,
                    &mut control_flow,
                    &mut callback,
                );
            }

            // The purpose of the back buffer and that swap is to not hold borrow_mut when
            // we're doing callback to the user, since we can double borrow if the user decides
            // to create a window in one of those callbacks.
            std::mem::swap(
                &mut sctk_event_sink_back_buffer,
                &mut self.state.sctk_events,
            );

            // handle a11y events
            #[cfg(feature = "a11y")]
            if let Ok(mut events) = self.a11y_events.lock() {
                for event in events.drain(..) {
                    match event {
                        adapter::A11yWrapper::Enabled => sticky_exit_callback(
                            IcedSctkEvent::A11yEnabled,
                            &self.state,
                            &mut control_flow,
                            &mut callback,
                        ),
                        adapter::A11yWrapper::Event(event) => {
                            sticky_exit_callback(
                                IcedSctkEvent::A11yEvent(event),
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            )
                        }
                    }
                }
            }
            // Handle pending sctk events.
            for event in sctk_event_sink_back_buffer.drain(..) {
                match event {
                    SctkEvent::PointerEvent { ref variant, .. } => {
                        let surface_id = variant.surface.id();

                        cursor_position.insert(
                            surface_id,
                            LogicalPosition::new(
                                variant.position.0,
                                variant.position.1,
                            ),
                        );
                    }

                    SctkEvent::PopupEvent {
                        variant: PopupEventVariant::Done,
                        toplevel_id,
                        parent_id,
                        id,
                    } => {
                        match self
                            .state
                            .popups
                            .iter()
                            .position(|s| s.popup.wl_surface().id() == id.id())
                        {
                            Some(p) => {
                                let _p = self.state.popups.remove(p);
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(
                                        SctkEvent::PopupEvent {
                                            variant: PopupEventVariant::Done,
                                            toplevel_id,
                                            parent_id,
                                            id,
                                        },
                                    ),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                            None => (),
                        };

                        continue;
                    }

                    SctkEvent::LayerSurfaceEvent {
                        variant: LayerSurfaceEventVariant::Done,
                        id,
                    } => {
                        cursor_position.remove(&id.id());

                        if let Some(i) =
                            self.state.layer_surfaces.iter().position(|l| {
                                l.surface.wl_surface().id() == id.id()
                            })
                        {
                            let _l = self.state.layer_surfaces.remove(i);
                            sticky_exit_callback(
                                IcedSctkEvent::SctkEvent(
                                    SctkEvent::LayerSurfaceEvent {
                                        variant: LayerSurfaceEventVariant::Done,
                                        id,
                                    },
                                ),
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            );
                        }

                        continue;
                    }

                    SctkEvent::WindowEvent {
                        variant: WindowEventVariant::Close,
                        id,
                    } => {
                        if let Some(i) =
                            self.state.windows.iter().position(|l| {
                                l.window.wl_surface().id() == id.id()
                            })
                        {
                            let w = self.state.windows.remove(i);
                            w.window.xdg_toplevel().destroy();
                            sticky_exit_callback(
                                IcedSctkEvent::SctkEvent(
                                    SctkEvent::WindowEvent {
                                        variant: WindowEventVariant::Close,
                                        id,
                                    },
                                ),
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            );
                        }

                        continue;
                    }
                    _ => (),
                }

                sticky_exit_callback(
                    IcedSctkEvent::SctkEvent(event),
                    &self.state,
                    &mut control_flow,
                    &mut callback,
                )
            }

            // handle events indirectly via callback to the user.
            let (sctk_events, user_events): (Vec<_>, Vec<_>) = self
                .state
                .pending_user_events
                .drain(..)
                .partition(|e| matches!(e, Event::SctkEvent(_)));
            let mut to_commit = HashMap::new();
            let mut pending_redraws = Vec::new();
            for event in sctk_events.into_iter().chain(user_events.into_iter())
            {
                match event {
                    Event::Message(m) => {
                        sticky_exit_callback(
                            IcedSctkEvent::UserEvent(m),
                            &self.state,
                            &mut control_flow,
                            &mut callback,
                        );
                    }
                    Event::SctkEvent(event) => {
                        match event {
                            IcedSctkEvent::RedrawRequested(id) => {
                                pending_redraws.push(id);
                            },
                            e => sticky_exit_callback(
                                e,
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            ),
                        }
                    }
                    Event::LayerSurface(action) => match action {
                        platform_specific::wayland::layer_surface::Action::LayerSurface {
                            builder,
                            _phantom,
                        } => {
                            // TODO ASHLEY: error handling
                            if let Ok((id, wl_surface)) = self.state.get_layer_surface(builder) {
                                let object_id = wl_surface.id();
                                // TODO Ashley: all surfaces should probably have an optional title for a11y if nothing else
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::LayerSurfaceEvent {
                                        variant: LayerSurfaceEventVariant::Created(object_id.clone(), id),
                                        id: wl_surface.clone(),
                                    }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                                #[cfg(feature = "a11y")]
                                {
                                    let adapter = self.init_a11y_adapter(&wl_surface, None, None, iced_accessibility::accesskit::Role::Window);

                                    sticky_exit_callback(
                                        IcedSctkEvent::A11ySurfaceCreated(SurfaceIdWrapper::LayerSurface(id), adapter),
                                        &self.state,
                                        &mut control_flow,
                                        &mut callback,
                                    );
                                }
                            }
                        }
                        platform_specific::wayland::layer_surface::Action::Size {
                            id,
                            width,
                            height,
                        } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.set_size(width, height);
                                pending_redraws.push(layer_surface.surface.wl_surface().id());
                                    let wl_surface = layer_surface.surface.wl_surface();

                                if let Some(mut prev_configure) = layer_surface.last_configure.clone() {
                                    prev_configure.new_size = (width.unwrap_or(prev_configure.new_size.0), width.unwrap_or(prev_configure.new_size.1));
                                    sticky_exit_callback(
                                        IcedSctkEvent::SctkEvent(SctkEvent::LayerSurfaceEvent { variant: LayerSurfaceEventVariant::Configure(prev_configure, wl_surface.clone(), false), id: wl_surface.clone()}),
                                        &self.state,
                                        &mut control_flow,
                                        &mut callback,
                                    );
                                }
                            }
                        },
                        platform_specific::wayland::layer_surface::Action::Destroy(id) => {
                            if let Some(i) = self.state.layer_surfaces.iter().position(|l| l.id == id) {
                                let l = self.state.layer_surfaces.remove(i);
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::LayerSurfaceEvent {
                                        variant: LayerSurfaceEventVariant::Done,
                                        id: l.surface.wl_surface().clone(),
                                    }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        },
                        platform_specific::wayland::layer_surface::Action::Anchor { id, anchor } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.anchor = anchor;
                                layer_surface.surface.set_anchor(anchor);
                                to_commit.insert(id, layer_surface.surface.wl_surface().clone());

                            }
                        }
                        platform_specific::wayland::layer_surface::Action::ExclusiveZone {
                            id,
                            exclusive_zone,
                        } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.exclusive_zone = exclusive_zone;
                                layer_surface.surface.set_exclusive_zone(exclusive_zone);
                                to_commit.insert(id, layer_surface.surface.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::layer_surface::Action::Margin {
                            id,
                            margin,
                        } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.margin = margin;
                                layer_surface.surface.set_margin(margin.top, margin.right, margin.bottom, margin.left);
                                to_commit.insert(id, layer_surface.surface.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::layer_surface::Action::KeyboardInteractivity { id, keyboard_interactivity } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.keyboard_interactivity = keyboard_interactivity;
                                layer_surface.surface.set_keyboard_interactivity(keyboard_interactivity);
                                to_commit.insert(id, layer_surface.surface.wl_surface().clone());

                            }
                        },
                        platform_specific::wayland::layer_surface::Action::Layer { id, layer } => {
                            if let Some(layer_surface) = self.state.layer_surfaces.iter_mut().find(|l| l.id == id) {
                                layer_surface.layer = layer;
                                layer_surface.surface.set_layer(layer);
                                to_commit.insert(id, layer_surface.surface.wl_surface().clone());

                            }
                        },
                    },
                    Event::SetCursor(iced_icon) => {
                        if let Some(seat) = self.state.seats.get_mut(0) {
                            let icon = conversion::cursor_icon(iced_icon);
                            seat.icon = Some(icon);
                            seat.set_cursor(self.wayland_dispatcher.as_source_ref().connection(), icon);
                        }

                    }
                    Event::Window(action) => match action {
                        platform_specific::wayland::window::Action::Window { builder, _phantom } => {
                            #[cfg(feature = "a11y")]
                            let app_id = builder.app_id.clone();
                            #[cfg(feature = "a11y")]
                            let title = builder.title.clone();
                            let (id, wl_surface) = self.state.get_window(builder);
                            let object_id = wl_surface.id();
                            sticky_exit_callback(
                                IcedSctkEvent::SctkEvent(SctkEvent::WindowEvent {
                                    variant: WindowEventVariant::Created(object_id.clone(), id),
                                    id: wl_surface.clone() }),
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            );

                            #[cfg(feature = "a11y")]
                            {
                                let adapter = self.init_a11y_adapter(&wl_surface, app_id, title, iced_accessibility::accesskit::Role::Window);

                                sticky_exit_callback(
                                    IcedSctkEvent::A11ySurfaceCreated(SurfaceIdWrapper::Window(id), adapter),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        },
                        platform_specific::wayland::window::Action::Size { id, width, height } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.set_size(LogicalSize::new(NonZeroU32::new(width).unwrap_or(NonZeroU32::new(1).unwrap()), NonZeroU32::new(height).unwrap_or(NonZeroU32::new(1).unwrap())));
                                // TODO Ashley maybe don't force window size?
                                pending_redraws.push(window.window.wl_surface().id());
                                if window.last_configure.is_some() {
                                    sticky_exit_callback(
                                        IcedSctkEvent::SctkEvent(SctkEvent::WindowEvent { variant: WindowEventVariant::Size(window.current_size, window.window.wl_surface().clone(), false), id: window.window.wl_surface().clone()}),
                                        &self.state,
                                        &mut control_flow,
                                        &mut callback,
                                    );
                                }
                            }
                        },
                        platform_specific::wayland::window::Action::MinSize { id, size } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_min_size(size);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::MaxSize { id, size } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_max_size(size);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::Title { id, title } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_title(title);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::Minimize { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_minimized();
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::Maximize { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_maximized();
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::UnsetMaximize { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.unset_maximized();
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::Fullscreen { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                // TODO ASHLEY: allow specific output to be requested for fullscreen?
                                window.window.set_fullscreen(None);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::UnsetFullscreen { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.unset_fullscreen();
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::InteractiveMove { id } => {
                            if let (Some(window), Some((seat, last_press))) = (self.state.windows.iter_mut().find(|w| w.id == id), self.state.seats.first().and_then(|seat| seat.last_ptr_press.map(|p| (&seat.seat, p.2)))) {
                                window.window.xdg_toplevel()._move(seat, last_press);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::InteractiveResize { id, edge } => {
                            if let (Some(window), Some((seat, last_press))) = (self.state.windows.iter_mut().find(|w| w.id == id), self.state.seats.first().and_then(|seat| seat.last_ptr_press.map(|p| (&seat.seat, p.2)))) {
                                window.window.xdg_toplevel().resize(seat, last_press, edge);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::ToggleMaximized { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                if let Some(c) = &window.last_configure {
                                    if c.is_maximized() {
                                        window.window.unset_maximized();
                                    } else {
                                        window.window.set_maximized();
                                    }
                                    to_commit.insert(id, window.window.wl_surface().clone());
                                }
                            }
                        },
                        platform_specific::wayland::window::Action::ShowWindowMenu { id } => {
                            if let (Some(window), Some((seat, last_press))) = (self.state.windows.iter_mut().find(|w| w.id == id), self.state.seats.first().and_then(|seat| seat.last_ptr_press.map(|p| (&seat.seat, p.2)))) {
                                let surface_id = window.window.wl_surface().id();

                                let cursor_position = cursor_position.get(&surface_id)
                                    .cloned()
                                    .unwrap_or_default();

                                // Cursor position does not need to be scaled here.
                                let PhysicalPosition { x, y } = cursor_position.to_physical::<i32>(1.0);

                                window.window.xdg_toplevel().show_window_menu(seat, last_press, x as i32, y as i32);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::Destroy(id) => {
                            if let Some(i) = self.state.windows.iter().position(|l| l.id == id) {
                                let window = self.state.windows.remove(i);
                                window.window.xdg_toplevel().destroy();
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::WindowEvent {
                                        variant: WindowEventVariant::Close,
                                        id: window.window.wl_surface().clone(),
                                    }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        },
                        platform_specific::wayland::window::Action::Mode(id, mode) => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                match mode {
                                    Mode::Windowed => {
                                        window.window.unset_fullscreen();
                                    },
                                    Mode::Fullscreen => {
                                        window.window.set_fullscreen(None);
                                    },
                                    Mode::Hidden => {
                                        window.window.set_minimized();
                                    },
                                }
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                        platform_specific::wayland::window::Action::ToggleFullscreen { id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                if let Some(c) = &window.last_configure {
                                    if c.is_fullscreen() {
                                        window.window.unset_fullscreen();
                                    } else {
                                        window.window.set_fullscreen(None);
                                    }
                                    to_commit.insert(id, window.window.wl_surface().clone());
                                }
                            }
                        },
                        platform_specific::wayland::window::Action::AppId { id, app_id } => {
                            if let Some(window) = self.state.windows.iter_mut().find(|w| w.id == id) {
                                window.window.set_app_id(app_id);
                                to_commit.insert(id, window.window.wl_surface().clone());
                            }
                        },
                    },
                    Event::Popup(action) => match action {
                        platform_specific::wayland::popup::Action::Popup { popup, .. } => {
                            if let Ok((id, parent_id, toplevel_id, wl_surface)) = self.state.get_popup(popup) {
                                let object_id = wl_surface.id();
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::PopupEvent {
                                        variant: crate::sctk_event::PopupEventVariant::Created(object_id.clone(), id),
                                        toplevel_id, parent_id, id: wl_surface.clone() }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );

                                #[cfg(feature = "a11y")]
                                {
                                let adapter = self.init_a11y_adapter(&wl_surface, None, None, iced_accessibility::accesskit::Role::Window);

                                sticky_exit_callback(
                                    IcedSctkEvent::A11ySurfaceCreated(SurfaceIdWrapper::LayerSurface(id), adapter),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                            }
                        },
                        // XXX popup destruction must be done carefully
                        // first destroy the uppermost popup, then work down to the requested popup
                        platform_specific::wayland::popup::Action::Destroy { id } => {
                            let sctk_popup = match self.state
                                .popups
                                .iter()
                                .position(|s| s.data.id == id)
                            {
                                Some(p) => self.state.popups.remove(p),
                                None => continue,
                            };
                            let mut to_destroy = vec![sctk_popup];
                            while let Some(popup_to_destroy) = to_destroy.last() {
                                match popup_to_destroy.data.parent.clone() {
                                    state::SctkSurface::LayerSurface(_) | state::SctkSurface::Window(_) => {
                                        break;
                                    }
                                    state::SctkSurface::Popup(popup_to_destroy_first) => {
                                        let popup_to_destroy_first = self
                                            .state
                                            .popups
                                            .iter()
                                            .position(|p| p.popup.wl_surface() == &popup_to_destroy_first)
                                            .unwrap();
                                        let popup_to_destroy_first = self.state.popups.remove(popup_to_destroy_first);
                                        to_destroy.push(popup_to_destroy_first);
                                    }
                                }
                            }
                            for popup in to_destroy.into_iter().rev() {
                                sticky_exit_callback(IcedSctkEvent::SctkEvent(SctkEvent::PopupEvent {
                                    variant: PopupEventVariant::Done,
                                    toplevel_id: popup.data.toplevel.clone(),
                                    parent_id: popup.data.parent.wl_surface().clone(),
                                    id: popup.popup.wl_surface().clone(),
                                }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        },
                        platform_specific::wayland::popup::Action::Size { id, width, height } => {
                            if let Some(sctk_popup) = self.state
                                .popups
                                .iter_mut()
                                .find(|s| s.data.id == id)
                            {
                                // update geometry
                                // update positioner
                                self.state.token_ctr += 1;
                                sctk_popup.set_size(width, height, self.state.token_ctr);

                                pending_redraws.push(sctk_popup.popup.wl_surface().id());

                                sticky_exit_callback(IcedSctkEvent::SctkEvent(SctkEvent::PopupEvent {
                                    variant: PopupEventVariant::Size(width, height),
                                    toplevel_id: sctk_popup.data.toplevel.clone(),
                                    parent_id: sctk_popup.data.parent.wl_surface().clone(),
                                    id: sctk_popup.popup.wl_surface().clone(),
                                }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        },
                        // TODO probably remove this?
                        platform_specific::wayland::popup::Action::Grab { .. } => {},
                    },
                    Event::DataDevice(action) => {
                        match action.inner {
                            platform_specific::wayland::data_device::ActionInner::Accept(mime_type) => {
                                let drag_offer = match self.state.dnd_offer.as_mut().and_then(|o| o.offer.as_ref()) {
                                    Some(d) => d,
                                    None => continue,
                                };
                                drag_offer.accept_mime_type(drag_offer.serial, mime_type);
                            }
                            platform_specific::wayland::data_device::ActionInner::StartInternalDnd { origin_id, icon_id } => {
                                let qh = &self.state.queue_handle.clone();
                                let seat = match self.state.seats.get(0) {
                                    Some(s) => s,
                                    None => continue,
                                };
                                let serial = match seat.last_ptr_press {
                                    Some(s) => s.2,
                                    None => continue,
                                };

                                let origin = match self
                                .state
                                .windows
                                .iter()
                                .find(|w| w.id == origin_id)
                                .map(|w| Some(w.window.wl_surface()))
                                .unwrap_or_else(|| self.state.layer_surfaces.iter()
                                                .find(|l| l.id == origin_id).map(|l| Some(l.surface.wl_surface()))
                                .unwrap_or_else(|| self.state.popups.iter().find(|p| p.data.id == origin_id).map(|p| p.popup.wl_surface()))) {
                                    Some(s) => s.clone(),
                                    None => continue,
                                };
                                let device = match self.state.seats.get(0) {
                                    Some(s) => &s.data_device,
                                    None => continue,
                                };
                                let icon_surface =  if let Some(icon_id) = icon_id{
                                    let wl_surface = self.state.compositor_state.create_surface(qh);
                                    DragSource::start_internal_drag(device, &origin, Some(&wl_surface), serial);
                                    Some((wl_surface, icon_id))
                                } else {
                                    DragSource::start_internal_drag(device, &origin, None, serial);
                                    None
                                };
                                self.state.dnd_source = Some(Dnd {
                                    origin_id,
                                    icon_surface,
                                    origin,
                                    source: None,
                                    pending_requests: Vec::new(),
                                    pipe: None,
                                    cur_write: None,
                                });
                            }
                            platform_specific::wayland::data_device::ActionInner::StartDnd { mime_types, actions, origin_id, icon_id, data } => {
                                if let Some(dnd_source) = self.state.dnd_source.as_ref() {
                                    if dnd_source.cur_write.is_some() {
                                        continue;
                                    }
                                }
                                let qh = &self.state.queue_handle.clone();
                                let seat = match self.state.seats.get(0) {
                                    Some(s) => s,
                                    None => continue,
                                };
                                // Get last pointer press or touch down serial, whichever is newer
                                let Some(serial) = seat.last_ptr_press.map(|s| s.2).max(seat.last_touch_down.map(|s| s.2)) else {
                                    continue;
                                };

                                let origin = match self
                                .state
                                .windows
                                .iter()
                                .find(|w| w.id == origin_id)
                                .map(|w| Some(w.window.wl_surface()))
                                .unwrap_or_else(|| self.state.layer_surfaces.iter()
                                                .find(|l| l.id == origin_id).map(|l| Some(l.surface.wl_surface()))
                                .unwrap_or_else(|| self.state.popups.iter().find(|p| p.data.id == origin_id).map(|p| p.popup.wl_surface()))) {
                                    Some(s) => s.clone(),
                                    None => continue,
                                };
                                let device = match self.state.seats.get(0) {
                                    Some(s) => &s.data_device,
                                    None => continue,
                                };
                                let source = self.state.data_device_manager_state.create_drag_and_drop_source(qh, mime_types.iter().map(|s| s.as_str()).collect::<Vec<_>>(), actions);
                                let icon_surface =  if let Some((icon_id, offset)) = icon_id{
                                    let icon_native_id = match &icon_id {
                                        DndIcon::Custom(icon_id) => *icon_id,
                                        DndIcon::Widget(icon_id, _) => *icon_id,
                                    };
                                    let wl_surface = self.state.compositor_state.create_surface(qh);
                                    if offset != crate::core::Vector::ZERO {
                                        wl_surface.offset(offset.x as i32, offset.y as i32);
                                    }
                                    source.start_drag(device, &origin, Some(&wl_surface), serial);
                                    sticky_exit_callback(
                                        IcedSctkEvent::DndSurfaceCreated(
                                                    wl_surface.clone(),
                                                    icon_id,
                                                    origin_id)
                                                ,
                                            &self.state,
                                            &mut control_flow,
                                            &mut callback
                                    );
                                   Some((wl_surface, icon_native_id))
                                } else {
                                    source.start_drag(device, &origin, None, serial);
                                    None
                                };
                                self.state.dnd_source = Some(Dnd { origin_id, origin, source: Some((source, data)), icon_surface, pending_requests: Vec::new(), pipe: None, cur_write: None });
                            },
                            platform_specific::wayland::data_device::ActionInner::DndFinished => {
                                if let Some(offer) = self.state.dnd_offer.take().filter(|o| o.offer.is_some()) {
                                    if offer.dropped {
                                        offer.offer.unwrap().finish();
                                    }
                                    else {
                                        self.state.dnd_offer = Some(offer);
                                    }
                               }
                            },
                            platform_specific::wayland::data_device::ActionInner::DndCancelled => {
                                if let Some(mut source) = self.state.dnd_source.take() {
                                    if let Some(s) = source.icon_surface.take() {
                                        s.0.destroy();
                                    }
                                    sticky_exit_callback(
                                        IcedSctkEvent::SctkEvent(SctkEvent::DataSource(DataSourceEvent::DndCancelled)),
                                            &self.state,
                                            &mut control_flow,
                                            &mut callback
                                    );
                                }
                            },
                            platform_specific::wayland::data_device::ActionInner::RequestDndData (mime_type) => {
                                if let Some(dnd_offer) = self.state.dnd_offer.as_mut() {
                                    let Some(offer) = dnd_offer.offer.as_ref() else {
                                        continue;
                                    };
                                    let read_pipe = match offer.receive(mime_type.clone()) {
                                        Ok(p) => p,
                                        Err(_) => continue, // TODO error handling
                                    };
                                    let loop_handle = self.event_loop.handle();
                                    match self.event_loop.handle().insert_source(read_pipe, move |_, f, state| {
                                        let mut dnd_offer = match state.dnd_offer.take() {
                                            Some(s) => s,
                                            None => return PostAction::Continue,
                                        };
                                        let Some(offer) = dnd_offer.offer.as_ref() else {
                                            return PostAction::Remove;
                                        };
                                        let (mime_type, data, token) = match dnd_offer.cur_read.take() {
                                            Some(s) => s,
                                            None => return PostAction::Continue,
                                        };
                                        let mut reader = BufReader::new(f.as_ref());
                                        let consumed = match reader.fill_buf() {
                                            Ok(buf) => {
                                                if buf.is_empty() {
                                                    loop_handle.remove(token);
                                                    state.sctk_events.push(SctkEvent::DndOffer { event: DndOfferEvent::Data { data, mime_type }, surface: dnd_offer.surface.clone() });
                                                    if dnd_offer.dropped {
                                                        offer.finish();
                                                    } else {
                                                        state.dnd_offer = Some(dnd_offer);
                                                    }
                                                } else {
                                                    let mut data = data;
                                                    data.extend_from_slice(buf);
                                                    dnd_offer.cur_read = Some((mime_type, data, token));
                                                    state.dnd_offer = Some(dnd_offer);
                                                }
                                                buf.len()
                                            },
                                            Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted) => {
                                                dnd_offer.cur_read = Some((mime_type, data, token));
                                                state.dnd_offer = Some(dnd_offer);
                                                return PostAction::Continue;
                                            },
                                            Err(e) => {
                                                error!("Error reading selection data: {}", e);
                                                if !dnd_offer.dropped {
                                                    state.dnd_offer = Some(dnd_offer);
                                                }
                                                return PostAction::Remove;
                                            },
                                        };
                                        reader.consume(consumed);
                                        PostAction::Continue
                                    }) {
                                        Ok(token) => {
                                            dnd_offer.cur_read = Some((mime_type.clone(), Vec::new(), token));
                                        },
                                        Err(_) => continue,
                                    };
                                }
                            }
                            platform_specific::wayland::data_device::ActionInner::SetActions { preferred, accepted } => {
                                if let Some(offer) = self.state.dnd_offer.as_ref().and_then(|o| o.offer.as_ref()) {
                                    offer.set_actions(accepted, preferred);
                                }
                            }
                        }
                    },
                    Event::Activation(activation_event) => match activation_event {
                        platform_specific::wayland::activation::Action::RequestToken { app_id, window, message } => {
                            if let Some(activation_state) = self.state.activation_state.as_ref() {
                                let (seat_and_serial, surface) = if let Some(id) = window {
                                    let surface = self.state.windows.iter().find(|w| w.id == id)
                                        .map(|w| w.window.wl_surface().clone())
                                        .or_else(|| self.state.layer_surfaces.iter().find(|l| l.id == id)
                                            .map(|l| l.surface.wl_surface().clone())
                                        );
                                    let seat_and_serial = surface.as_ref().and_then(|surface| {
                                        self.state.seats.first().and_then(|seat| if seat.kbd_focus.as_ref().map(|focus| focus == surface).unwrap_or(false) {
                                            seat.last_kbd_press.as_ref().map(|(_, serial)| (seat.seat.clone(), *serial))
                                        } else if seat.ptr_focus.as_ref().map(|focus| focus == surface).unwrap_or(false) {
                                            seat.last_ptr_press.as_ref().map(|(_, _, serial)| (seat.seat.clone(), *serial))
                                        } else {
                                            None
                                        })
                                    });

                                    (seat_and_serial, surface)
                                } else {
                                    (None, None)
                                };

                                activation_state.request_token_with_data(&self.state.queue_handle, IcedRequestData::new(
                                    RequestData {
                                        app_id,
                                        seat_and_serial,
                                        surface,
                                    },
                                    message,
                                ));
                            } else {
                                // if we don't have the global, we don't want to stall the app
                                sticky_exit_callback(
                                    IcedSctkEvent::UserEvent(message(None)),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                )
                            }
                        },
                        platform_specific::wayland::activation::Action::Activate { window, token } => {
                            if let Some(activation_state) = self.state.activation_state.as_ref() {
                                if let Some(surface) = self.state.windows.iter().find(|w| w.id == window).map(|w| w.window.wl_surface()) {
                                    activation_state.activate::<SctkState<T>>(surface, token)
                                }
                            }
                        },
                    },
                    Event::SessionLock(action) => match action {
                        platform_specific::wayland::session_lock::Action::Lock => {
                            if self.state.session_lock.is_none() {
                                // TODO send message on error? When protocol doesn't exist.
                                self.state.session_lock = self.state.session_lock_state.lock(&self.state.queue_handle).ok();
                            }
                        }
                        platform_specific::wayland::session_lock::Action::Unlock => {
                            if let Some(session_lock) = self.state.session_lock.take() {
                                session_lock.unlock();
                            }
                            // Make sure server processes unlock before client exits
                            let _ = self.state.connection.roundtrip();
                            sticky_exit_callback(
                                IcedSctkEvent::SctkEvent(SctkEvent::SessionUnlocked),
                                &self.state,
                                &mut control_flow,
                                &mut callback,
                            );
                        }
                        platform_specific::wayland::session_lock::Action::LockSurface { id, output, _phantom } => {
                            // TODO how to handle this when there's no lock?
                            if let Some(surface) = self.state.get_lock_surface(id, &output) {
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::SessionLockSurfaceCreated {surface, native_id: id}),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        }
                        platform_specific::wayland::session_lock::Action::DestroyLockSurface { id } => {
                            if let Some(i) =
                                self.state.lock_surfaces.iter().position(|s| {
                                    s.id == id
                                })
                            {
                                let surface = self.state.lock_surfaces.remove(i);
                                sticky_exit_callback(
                                    IcedSctkEvent::SctkEvent(SctkEvent::SessionLockSurfaceDone {
                                        surface: surface.session_lock_surface.wl_surface().clone()
                                    }),
                                    &self.state,
                                    &mut control_flow,
                                    &mut callback,
                                );
                            }
                        }
                    }
                }
            }

            // Send events cleared.
            sticky_exit_callback(
                IcedSctkEvent::MainEventsCleared,
                &self.state,
                &mut control_flow,
                &mut callback,
            );

            // redraw
            pending_redraws.dedup();
            for id in pending_redraws {
                sticky_exit_callback(
                    IcedSctkEvent::RedrawRequested(id.clone()),
                    &self.state,
                    &mut control_flow,
                    &mut callback,
                );
            }

            // commit changes made via actions
            for s in to_commit {
                s.1.commit();
            }

            // Send RedrawEventCleared.
            sticky_exit_callback(
                IcedSctkEvent::RedrawEventsCleared,
                &self.state,
                &mut control_flow,
                &mut callback,
            );
        };

        callback(IcedSctkEvent::LoopDestroyed, &self.state, &mut control_flow);
        exit_code
    }
}

fn sticky_exit_callback<T, F>(
    evt: IcedSctkEvent<T>,
    target: &SctkState<T>,
    control_flow: &mut ControlFlow,
    callback: &mut F,
) where
    F: FnMut(IcedSctkEvent<T>, &SctkState<T>, &mut ControlFlow),
{
    // make ControlFlow::ExitWithCode sticky by providing a dummy
    // control flow reference if it is already ExitWithCode.
    if let ControlFlow::ExitWithCode(code) = *control_flow {
        callback(evt, target, &mut ControlFlow::ExitWithCode(code))
    } else {
        callback(evt, target, control_flow)
    }
}

fn raw_os_err(err: calloop::Error) -> i32 {
    match err {
        calloop::Error::IoError(err) => err.raw_os_error(),
        _ => None,
    }
    .unwrap_or(1)
}
