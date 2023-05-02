//! Create interactive, native cross-platform applications for WGPU.
#[path = "application/drag_resize.rs"]
mod drag_resize;
mod state;
mod window_manager;

use crate::application::UserEventWrapper;
use crate::conversion;
use crate::core;
use crate::core::clipboard::Kind;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::operation;
use crate::core::widget::Operation;
use crate::core::window;
use crate::core::Clipboard as CoreClipboard;
use crate::core::Length;
use crate::core::{Point, Size};
use crate::futures::futures::channel::mpsc;
use crate::futures::futures::channel::oneshot;
use crate::futures::futures::executor;
use crate::futures::futures::task;
use crate::futures::futures::{Future, StreamExt};
use crate::futures::{Executor, Runtime, Subscription};
use crate::graphics;
use crate::graphics::{compositor, Compositor};
use crate::multi_window::operation::OperationWrapper;
use crate::multi_window::window_manager::WindowManager;
use crate::runtime::command::{self, Command};
use crate::runtime::multi_window::Program;
use crate::runtime::user_interface::{self, UserInterface};
use crate::runtime::Debug;
use crate::{Clipboard, Error, Proxy, Settings};
use dnd::DndSurface;
use dnd::Icon;
use iced_graphics::Viewport;
use iced_runtime::futures::futures::FutureExt;
pub use state::State;
use window_clipboard::mime::ClipboardStoreData;
use winit::raw_window_handle::HasWindowHandle;

pub use crate::application::{default, Appearance, DefaultStyle};

use rustc_hash::FxHashMap;
use std::any::Any;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::time::Instant;

/// subscription mapper helper
pub fn subscription_map<A, E>(e: A::Message) -> UserEventWrapper<A::Message>
where
    A: Application,
    E: Executor,
    A::Theme: DefaultStyle,
{
    UserEventWrapper::Message(e)
}

/// An interactive, native, cross-platform, multi-windowed application.
///
/// This trait is the main entrypoint of multi-window Iced. Once implemented, you can run
/// your GUI application by simply calling [`run`]. It will run in
/// its own window.
///
/// An [`Application`] can execute asynchronous actions by returning a
/// [`Command`] in some of its methods.
///
/// When using an [`Application`] with the `debug` feature enabled, a debug view
/// can be toggled by pressing `F12`.
pub trait Application: Program
where
    Self::Theme: DefaultStyle,
{
    /// The data needed to initialize your [`Application`].
    type Flags;

    /// Initializes the [`Application`] with the flags provided to
    /// [`run`] as part of the [`Settings`].
    ///
    /// Here is where you should return the initial state of your app.
    ///
    /// Additionally, you can return a [`Command`] if you need to perform some
    /// async action in the background on startup. This is useful if you want to
    /// load state from a file, perform an initial HTTP request, etc.
    fn new(flags: Self::Flags) -> (Self, Command<Self::Message>);

    /// Returns the current title of the [`Application`].
    ///
    /// This title can be dynamic! The runtime will automatically update the
    /// title of your application when necessary.
    fn title(&self, window: window::Id) -> String;

    /// Returns the current `Theme` of the [`Application`].
    fn theme(&self, window: window::Id) -> Self::Theme;

    /// Returns the `Style` variation of the `Theme`.
    fn style(&self, theme: &Self::Theme) -> Appearance {
        theme.default_style()
    }

    /// Returns the event `Subscription` for the current state of the
    /// application.
    ///
    /// The messages produced by the `Subscription` will be handled by
    /// [`update`](#tymethod.update).
    ///
    /// A `Subscription` will be kept alive as long as you keep returning it!
    ///
    /// By default, it returns an empty subscription.
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }

    /// Returns the scale factor of the window of the [`Application`].
    ///
    /// It can be used to dynamically control the size of the UI at runtime
    /// (i.e. zooming).
    ///
    /// For instance, a scale factor of `2.0` will make widgets twice as big,
    /// while a scale factor of `0.5` will shrink them to half their size.
    ///
    /// By default, it returns `1.0`.
    #[allow(unused_variables)]
    fn scale_factor(&self, window: window::Id) -> f64 {
        1.0
    }
}

/// Runs an [`Application`] with an executor, compositor, and the provided
/// settings.
pub fn run<A, E, C>(
    settings: Settings<A::Flags>,
    graphics_settings: graphics::Settings,
) -> Result<(), Error>
where
    A: Application + 'static,
    E: Executor + 'static,
    C: Compositor<Renderer = A::Renderer> + 'static,
    A::Theme: DefaultStyle,
    A::Message: Send + 'static,
{
    use winit::event_loop::EventLoop;

    let mut debug = Debug::new();
    debug.startup_started();

    let event_loop = EventLoop::with_user_event()
        .build()
        .expect("Create event loop");

    let (proxy, worker) = Proxy::new(event_loop.create_proxy());

    let runtime = {
        let executor = E::new().map_err(Error::ExecutorCreationFailed)?;
        executor.spawn(worker);

        Runtime::new(executor, proxy.clone())
    };

    let (application, init_command) = {
        let flags = settings.flags;

        runtime.enter(|| A::new(flags))
    };

    let id = settings.id;
    let title = application.title(window::Id::MAIN);
    let should_main_be_visible = settings.window.visible;
    let exit_on_close_request = settings.window.exit_on_close_request;
    let resize_border = settings.window.resize_border;

    let (boot_sender, boot_receiver) = oneshot::channel();
    let (event_sender, event_receiver) = mpsc::unbounded();
    let (control_sender, control_receiver) = mpsc::unbounded();

    let instance = Box::pin(run_instance::<A, E, C>(
        application,
        runtime,
        proxy,
        debug,
        boot_receiver,
        event_receiver,
        control_sender,
        init_command,
        resize_border,
    ));

    let context = task::Context::from_waker(task::noop_waker_ref());

    struct Runner<Message: 'static, F, C> {
        instance: std::pin::Pin<Box<F>>,
        context: task::Context<'static>,
        boot: Option<BootConfig<C>>,
        sender: mpsc::UnboundedSender<Event<Message>>,
        receiver: mpsc::UnboundedReceiver<Control>,
        error: Option<Error>,
    }

    struct BootConfig<C> {
        sender: oneshot::Sender<Boot<C>>,
        id: Option<String>,
        title: String,
        window_settings: window::Settings,
        graphics_settings: graphics::Settings,
    }

    let mut runner = Runner {
        instance,
        context,
        boot: Some(BootConfig {
            sender: boot_sender,
            id,
            title,
            window_settings: settings.window,
            graphics_settings,
        }),
        sender: event_sender,
        receiver: control_receiver,
        error: None,
    };

    impl<Message, F, C> winit::application::ApplicationHandler<Message>
        for Runner<Message, F, C>
    where
        F: Future<Output = ()>,
        C: Compositor,
    {
        fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
            let Some(BootConfig {
                sender,
                id,
                title,
                window_settings,
                graphics_settings,
            }) = self.boot.take()
            else {
                return;
            };

            let should_be_visible = window_settings.visible;
            let exit_on_close_request = window_settings.exit_on_close_request;

            let window_attributes = conversion::window_attributes(
                window_settings,
                &title,
                event_loop.primary_monitor(),
                id,
            )
            .with_visible(false);

            log::debug!("Window attributes: {window_attributes:#?}");

            let window = match event_loop.create_window(window_attributes) {
                Ok(window) => Arc::new(window),
                Err(error) => {
                    self.error = Some(Error::WindowCreationFailed(error));
                    event_loop.exit();
                    return;
                }
            };

            let finish_boot = async move {
                let compositor =
                    C::new(graphics_settings, window.clone()).await?;

                sender
                    .send(Boot {
                        window,
                        compositor,
                        should_be_visible,
                        exit_on_close_request,
                    })
                    .ok()
                    .expect("Send boot event");

                Ok::<_, graphics::Error>(())
            };

            if let Err(error) = executor::block_on(finish_boot) {
                self.error = Some(Error::GraphicsCreationFailed(error));
                event_loop.exit();
            }
        }

        fn new_events(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            cause: winit::event::StartCause,
        ) {
            if self.boot.is_some() {
                return;
            }

            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::NewEvents(cause)),
            );
        }

        fn window_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            window_id: winit::window::WindowId,
            event: winit::event::WindowEvent,
        ) {
            #[cfg(target_os = "windows")]
            let is_move_or_resize = matches!(
                event,
                winit::event::WindowEvent::Resized(_)
                    | winit::event::WindowEvent::Moved(_)
            );

            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::WindowEvent {
                    window_id,
                    event,
                }),
            );

            // TODO: Remove when unnecessary
            // On Windows, we emulate an `AboutToWait` event after every `Resized` event
            // since the event loop does not resume during resize interaction.
            // More details: https://github.com/rust-windowing/winit/issues/3272
            #[cfg(target_os = "windows")]
            {
                if is_move_or_resize {
                    self.process_event(
                        event_loop,
                        Event::EventLoopAwakened(
                            winit::event::Event::AboutToWait,
                        ),
                    );
                }
            }
        }

        fn user_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            message: Message,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::UserEvent(
                    message,
                )),
            );
        }

        fn about_to_wait(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
        ) {
            self.process_event(
                event_loop,
                Event::EventLoopAwakened(winit::event::Event::AboutToWait),
            );
        }
    }

    impl<Message, F, C> Runner<Message, F, C>
    where
        F: Future<Output = ()>,
        C: Compositor,
    {
        fn process_event(
            &mut self,
            event_loop: &winit::event_loop::ActiveEventLoop,
            event: Event<Message>,
        ) {
            if event_loop.exiting() {
                return;
            }

            self.sender.start_send(event).expect("Send event");

            loop {
                let poll = self.instance.as_mut().poll(&mut self.context);

                match poll {
                    task::Poll::Pending => match self.receiver.try_next() {
                        Ok(Some(control)) => match control {
                            Control::ChangeFlow(flow) => {
                                use winit::event_loop::ControlFlow;

                                match (event_loop.control_flow(), flow) {
                                    (
                                        ControlFlow::WaitUntil(current),
                                        ControlFlow::WaitUntil(new),
                                    ) if new < current => {}
                                    (
                                        ControlFlow::WaitUntil(target),
                                        ControlFlow::Wait,
                                    ) if target > Instant::now() => {}
                                    _ => {
                                        event_loop.set_control_flow(flow);
                                    }
                                }
                            }
                            Control::CreateWindow {
                                id,
                                settings,
                                title,
                                monitor,
                            } => {
                                let exit_on_close_request =
                                    settings.exit_on_close_request;

                                let window = event_loop
                                    .create_window(
                                        conversion::window_attributes(
                                            settings, &title, monitor, None,
                                        ),
                                    )
                                    .expect("Create window");

                                self.process_event(
                                    event_loop,
                                    Event::WindowCreated {
                                        id,
                                        window,
                                        exit_on_close_request,
                                    },
                                );
                            }
                            Control::Exit => {
                                event_loop.exit();
                            }
                        },
                        _ => {
                            break;
                        }
                    },
                    task::Poll::Ready(_) => {
                        event_loop.exit();
                        break;
                    }
                };
            }
        }
    }

    let _ = event_loop.run_app(&mut runner);

    Ok(())
}

struct Boot<C> {
    window: Arc<winit::window::Window>,
    compositor: C,
    should_be_visible: bool,
    exit_on_close_request: bool,
}

enum Event<Message: 'static> {
    WindowCreated {
        id: window::Id,
        window: winit::window::Window,
        exit_on_close_request: bool,
    },
    EventLoopAwakened(winit::event::Event<Message>),
}

enum Control {
    ChangeFlow(winit::event_loop::ControlFlow),
    Exit,
    CreateWindow {
        id: window::Id,
        settings: window::Settings,
        title: String,
        monitor: Option<winit::monitor::MonitorHandle>,
    },
}

async fn run_instance<A, E, C>(
    mut application: A,
    mut runtime: Runtime<
        E,
        Proxy<UserEventWrapper<A::Message>>,
        UserEventWrapper<A::Message>,
    >,
    mut proxy: Proxy<UserEventWrapper<A::Message>>,
    mut debug: Debug,
    mut boot: oneshot::Receiver<Boot<C>>,
    mut event_receiver: mpsc::UnboundedReceiver<
        Event<UserEventWrapper<A::Message>>,
    >,
    mut control_sender: mpsc::UnboundedSender<Control>,
    init_command: Command<UserEventWrapper<A::Message>>,
    resize_border: u32,
) where
    A: Application + 'static,
    E: Executor + 'static,
    C: Compositor<Renderer = A::Renderer> + 'static,
    A::Theme: DefaultStyle,
    A::Message: Send + 'static,
{
    use winit::event;
    use winit::event_loop::ControlFlow;

    let Boot {
        window: main_window,
        mut compositor,
        should_be_visible,
        exit_on_close_request,
    } = boot.try_recv().ok().flatten().expect("Receive boot");

    let mut window_manager = WindowManager::new();

    let _ = window_manager.insert(
        window::Id::MAIN,
        main_window,
        &application,
        &mut compositor,
        exit_on_close_request,
    );

    let main_window = window_manager
        .get_mut(window::Id::MAIN)
        .expect("Get main window");

    if should_be_visible {
        main_window.raw.set_visible(true);
    }

    let mut clipboard =
        Clipboard::connect(&main_window.raw, Proxy::new(proxy.clone()));

    #[cfg(feature = "a11y")]
    let (window_a11y_id, adapter, mut a11y_enabled) = {
        let node_id = core::id::window_node_id();

        use iced_accessibility::accesskit::{
            NodeBuilder, NodeId, Role, Tree, TreeUpdate,
        };
        use iced_accessibility::accesskit_winit::Adapter;

        let title = main_window.raw.title().to_string();
        let proxy_clone = proxy.clone();
        (
            node_id,
            Adapter::new(
                &main_window.raw,
                move || {
                    let _ =
                        proxy_clone.send_event(UserEventWrapper::A11yEnabled);
                    let mut node = NodeBuilder::new(Role::Window);
                    node.set_name(title.clone());
                    let node = node.build(&mut iced_accessibility::accesskit::NodeClassSet::lock_global());
                    let root = NodeId(node_id);
                    TreeUpdate {
                        nodes: vec![(root, node)],
                        tree: Some(Tree::new(root)),
                        focus: root,
                    }
                },
                proxy.clone(),
            ),
            false,
        )
    };
    let mut events = {
        vec![(
            Some(window::Id::MAIN),
            core::Event::Window(window::Event::Opened {
                position: main_window.position(),
                size: main_window.size(),
            }),
        )]
    };

    let mut ui_caches = FxHashMap::default();
    let mut user_interfaces = ManuallyDrop::new(build_user_interfaces(
        &application,
        &mut debug,
        &mut window_manager,
        FxHashMap::from_iter([(
            window::Id::MAIN,
            user_interface::Cache::default(),
        )]),
        &mut clipboard,
    ));

    run_command(
        &application,
        &mut compositor,
        init_command,
        &mut runtime,
        &mut clipboard,
        &mut control_sender,
        &mut proxy,
        &mut debug,
        &mut window_manager,
        &mut ui_caches,
    );

    runtime.track(
        application
            .subscription()
            .map(subscription_map::<A, E>)
            .into_recipes(),
    );

    let mut messages = Vec::new();
    let mut user_events = 0;

    debug.startup_finished();

    let mut cur_dnd_surface: Option<window::Id> = None;

    'main: while let Some(event) = event_receiver.next().await {
        match event {
            Event::WindowCreated {
                id,
                window,
                exit_on_close_request,
            } => {
                let window = window_manager.insert(
                    id,
                    Arc::new(window),
                    &application,
                    &mut compositor,
                    exit_on_close_request,
                    resize_border,
                );

                let logical_size = window.state.logical_size();

                let _ = user_interfaces.insert(
                    id,
                    build_user_interface(
                        &application,
                        user_interface::Cache::default(),
                        &mut window.renderer,
                        logical_size,
                        &mut debug,
                        id,
                    ),
                );
                let _ = ui_caches.insert(id, user_interface::Cache::default());

                events.push((
                    Some(id),
                    core::Event::Window(window::Event::Opened {
                        position: window.position(),
                        size: window.size(),
                    }),
                ));
            }
            Event::EventLoopAwakened(event) => {
                match event {
                    event::Event::NewEvents(
                        event::StartCause::Init
                        | event::StartCause::ResumeTimeReached { .. },
                    ) => {
                        for (_id, window) in window_manager.iter_mut() {
                            // TODO once widgets can request to be redrawn, we can avoid always requesting a
                            // redraw
                            window.raw.request_redraw();
                        }
                    }
                    event::Event::PlatformSpecific(
                        event::PlatformSpecific::MacOS(
                            event::MacOS::ReceivedUrl(url),
                        ),
                    ) => {
                        use crate::core::event;

                        events.push((
                            None,
                            event::Event::PlatformSpecific(
                                event::PlatformSpecific::MacOS(
                                    event::MacOS::ReceivedUrl(url),
                                ),
                            ),
                        ));
                    }
                    event::Event::UserEvent(message) => {
                        messages.push(message);
                        user_events += 1;
                    }
                    event::Event::WindowEvent {
                        window_id: id,
                        event: event::WindowEvent::RedrawRequested,
                        ..
                    } => {
                        let Some((id, window)) =
                            window_manager.get_mut_alias(id)
                        else {
                            continue;
                        };

                        // TODO: Avoid redrawing all the time by forcing widgets to
                        // request redraws on state changes
                        //
                        // Then, we can use the `interface_state` here to decide if a redraw
                        // is needed right away, or simply wait until a specific time.
                        let redraw_event = core::Event::Window(
                            window::Event::RedrawRequested(Instant::now()),
                        );

                        let cursor = window.state.cursor();

                        let ui = user_interfaces
                            .get_mut(&id)
                            .expect("Get user interface");

                        let (ui_state, _) = ui.update(
                            &[redraw_event.clone()],
                            cursor,
                            &mut window.renderer,
                            &mut clipboard,
                            &mut messages,
                        );

                        debug.draw_started();
                        let new_mouse_interaction = ui.draw(
                            &mut window.renderer,
                            window.state.theme(),
                            &renderer::Style {
                                icon_color: window.state.icon_color(),
                                text_color: window.state.text_color(),
                                scale_factor: window.state.scale_factor(),
                            },
                            cursor,
                        );
                        debug.draw_finished();

                        if new_mouse_interaction != window.mouse_interaction {
                            window.raw.set_cursor(
                                conversion::mouse_interaction(
                                    new_mouse_interaction,
                                ),
                            );

                            window.mouse_interaction = new_mouse_interaction;
                        }

                        runtime.broadcast(
                            redraw_event.clone(),
                            core::event::Status::Ignored,
                            id,
                        );

                        let _ = control_sender.start_send(Control::ChangeFlow(
                            match ui_state {
                                user_interface::State::Updated {
                                    redraw_request: Some(redraw_request),
                                } => match redraw_request {
                                    window::RedrawRequest::NextFrame => {
                                        window.raw.request_redraw();

                                        ControlFlow::Wait
                                    }
                                    window::RedrawRequest::At(at) => {
                                        ControlFlow::WaitUntil(at)
                                    }
                                },
                                _ => ControlFlow::Wait,
                            },
                        ));

                        let physical_size = window.state.physical_size();

                        if physical_size.width == 0 || physical_size.height == 0
                        {
                            continue;
                        }

                        if window.viewport_version
                            != window.state.viewport_version()
                        {
                            let logical_size = window.state.logical_size();

                            debug.layout_started();
                            let ui = user_interfaces
                                .remove(&id)
                                .expect("Remove user interface");

                            let _ = user_interfaces.insert(
                                id,
                                ui.relayout(logical_size, &mut window.renderer),
                            );
                            debug.layout_finished();

                            debug.draw_started();
                            let new_mouse_interaction = user_interfaces
                                .get_mut(&id)
                                .expect("Get user interface")
                                .draw(
                                    &mut window.renderer,
                                    window.state.theme(),
                                    &renderer::Style {
                                        icon_color: window.state.icon_color(),
                                        text_color: window.state.text_color(),
                                        scale_factor: window
                                            .state
                                            .scale_factor(),
                                    },
                                    window.state.cursor(),
                                );
                            debug.draw_finished();

                            if new_mouse_interaction != window.mouse_interaction
                            {
                                window.raw.set_cursor(
                                    conversion::mouse_interaction(
                                        new_mouse_interaction,
                                    ),
                                );

                                window.mouse_interaction =
                                    new_mouse_interaction;
                            }

                            compositor.configure_surface(
                                &mut window.surface,
                                physical_size.width,
                                physical_size.height,
                            );

                            window.viewport_version =
                                window.state.viewport_version();
                        }

                        debug.render_started();
                        match compositor.present(
                            &mut window.renderer,
                            &mut window.surface,
                            window.state.viewport(),
                            window.state.background_color(),
                            &debug.overlay(),
                        ) {
                            Ok(()) => {
                                debug.render_finished();

                                // TODO: Handle animations!
                                // Maybe we can use `ControlFlow::WaitUntil` for this.
                            }
                            Err(error) => match error {
                                // This is an unrecoverable error.
                                compositor::SurfaceError::OutOfMemory => {
                                    panic!("{:?}", error);
                                }
                                _ => {
                                    debug.render_finished();

                                    log::error!(
                                        "Error {error:?} when \
                                        presenting surface."
                                    );

                                    // Try rendering all windows again next frame.
                                    for (_id, window) in
                                        window_manager.iter_mut()
                                    {
                                        window.raw.request_redraw();
                                    }
                                }
                            },
                        }
                    }
                    event::Event::WindowEvent {
                        event: window_event,
                        window_id,
                    } => {
                        let Some((id, window)) =
                            window_manager.get_mut_alias(window_id)
                        else {
                            continue;
                        };

                        // Initiates a drag resize window state when found.
                        if let Some(func) =
                            window.drag_resize_window_func.as_mut()
                        {
                            if func(&window.raw, &window_event) {
                                continue;
                            }
                        }

                        if matches!(
                            window_event,
                            winit::event::WindowEvent::CloseRequested
                        ) && window.exit_on_close_request
                        {
                            let w = window_manager.remove(id);
                            let _ = user_interfaces.remove(&id);
                            let _ = ui_caches.remove(&id);
                            // XXX Empty rectangle list un-registers the window
                            if let Some(w) = w {
                                clipboard.register_dnd_destination(
                                    DndSurface(Arc::new(Box::new(
                                        w.raw.clone(),
                                    ))),
                                    Vec::new(),
                                );
                            }
                            events.push((
                                None,
                                core::Event::Window(window::Event::Closed),
                            ));

                            if window_manager.is_empty() {
                                break 'main;
                            }
                        } else {
                            window.state.update(
                                &window.raw,
                                &window_event,
                                &mut debug,
                            );

                            if let Some(event) = conversion::window_event(
                                window_event,
                                window.state.scale_factor(),
                                window.state.modifiers(),
                            ) {
                                events.push((Some(id), event));
                            }
                        }
                    }
                    event::Event::AboutToWait => {
                        if events.is_empty() && messages.is_empty() {
                            continue;
                        }

                        debug.event_processing_started();
                        let mut uis_stale = false;

                        for (id, window) in window_manager.iter_mut() {
                            let mut window_events = vec![];

                            events.retain(|(window_id, event)| {
                                if *window_id == Some(id) || window_id.is_none()
                                {
                                    window_events.push(event.clone());
                                    false
                                } else {
                                    true
                                }
                            });

                            if window_events.is_empty() && messages.is_empty() {
                                continue;
                            }

                            let (ui_state, statuses) = user_interfaces
                                .get_mut(&id)
                                .expect("Get user interface")
                                .update(
                                    &window_events,
                                    window.state.cursor(),
                                    &mut window.renderer,
                                    &mut clipboard,
                                    &mut messages,
                                );

                            window.raw.request_redraw();

                            if !uis_stale {
                                uis_stale = matches!(
                                    ui_state,
                                    user_interface::State::Outdated
                                );
                            }

                            for (event, status) in window_events
                                .into_iter()
                                .zip(statuses.into_iter())
                            {
                                runtime.broadcast(event, status, id);
                            }
                        }

                        debug.event_processing_finished();

                        // TODO mw application update returns which window IDs to update
                        if !messages.is_empty() || uis_stale {
                            let mut cached_interfaces: FxHashMap<
                                window::Id,
                                user_interface::Cache,
                            > = ManuallyDrop::into_inner(user_interfaces)
                                .drain()
                                .map(|(id, ui)| (id, ui.into_cache()))
                                .collect();

                            // Update application
                            update(
                                &mut application,
                                &mut compositor,
                                &mut runtime,
                                &mut clipboard,
                                &mut control_sender,
                                &mut proxy,
                                &mut debug,
                                &mut messages,
                                &mut window_manager,
                                &mut cached_interfaces,
                            );

                            // we must synchronize all window states with application state after an
                            // application update since we don't know what changed
                            for (id, window) in window_manager.iter_mut() {
                                window.state.synchronize(
                                    &application,
                                    id,
                                    &window.raw,
                                );

                                // TODO once widgets can request to be redrawn, we can avoid always requesting a
                                // redraw
                                window.raw.request_redraw();
                            }

                            // rebuild UIs with the synchronized states
                            user_interfaces =
                                ManuallyDrop::new(build_user_interfaces(
                                    &application,
                                    &mut debug,
                                    &mut window_manager,
                                    cached_interfaces,
                                    &mut clipboard,
                                ));

                            if user_events > 0 {
                                proxy.free_slots(user_events);
                                user_events = 0;
                            }
                        }

                        debug.draw_started();

                        for (id, window) in window_manager.iter_mut() {
                            // TODO: Avoid redrawing all the time by forcing widgets to
                            //  request redraws on state changes
                            //
                            // Then, we can use the `interface_state` here to decide if a redraw
                            // is needed right away, or simply wait until a specific time.
                            let redraw_event = core::Event::Window(
                                id,
                                window::Event::RedrawRequested(Instant::now()),
                            );

                            let cursor = window.state.cursor();

                            let ui = user_interfaces
                                .get_mut(&id)
                                .expect("Get user interface");

                            let (ui_state, _) = ui.update(
                                &[redraw_event.clone()],
                                cursor,
                                &mut window.renderer,
                                &mut clipboard,
                                &mut messages,
                            );

                            let new_mouse_interaction = {
                                let state = &window.state;

                                ui.draw(
                                    &mut window.renderer,
                                    state.theme(),
                                    &renderer::Style {
                                        icon_color: state.icon_color(),
                                        text_color: state.text_color(),
                                        scale_factor: state.scale_factor(),
                                    },
                                    cursor,
                                )
                            };

                            if new_mouse_interaction != window.mouse_interaction
                            {
                                window.raw.set_cursor_icon(
                                    conversion::mouse_interaction(
                                        new_mouse_interaction,
                                    ),
                                );

                                window.mouse_interaction =
                                    new_mouse_interaction;
                            }

                            // TODO once widgets can request to be redrawn, we can avoid always requesting a
                            // redraw
                            window.raw.request_redraw();

                            runtime.broadcast(
                                redraw_event.clone(),
                                core::event::Status::Ignored,
                                id,
                            );

                            let _ = control_sender.start_send(
                                Control::ChangeFlow(match ui_state {
                                    user_interface::State::Updated {
                                        redraw_request: Some(redraw_request),
                                    } => match redraw_request {
                                        window::RedrawRequest::NextFrame => {
                                            ControlFlow::Poll
                                        }
                                        window::RedrawRequest::At(at) => {
                                            ControlFlow::WaitUntil(at)
                                        }
                                    },
                                    _ => ControlFlow::Wait,
                                }),
                            );
                        }

                        debug.draw_finished();
                    }
                    event::Event::PlatformSpecific(
                        event::PlatformSpecific::MacOS(
                            event::MacOS::ReceivedUrl(url),
                        ),
                    ) => {
                        use crate::core::event;

                        events.push((
                            None,
                            event::Event::PlatformSpecific(
                                event::PlatformSpecific::MacOS(
                                    event::MacOS::ReceivedUrl(url),
                                ),
                            ),
                        ));
                    }
                    event::Event::UserEvent(message) => {
                        match message {
                            UserEventWrapper::Message(m) => messages.push(m),
                            #[cfg(feature = "a11y")]
                            UserEventWrapper::A11y(request) => {
                                match request.request.action {
                                    iced_accessibility::accesskit::Action::Focus => {
                                        // TODO send a command for this
                                     }
                                     _ => {}
                                 }
                                events.push((
                                    None,
                                    conversion::a11y(request.request),
                                ));
                            }
                            #[cfg(feature = "a11y")]
                            UserEventWrapper::A11yEnabled => {
                                a11y_enabled = true
                            }
                            UserEventWrapper::StartDnd {
                                internal,
                                source_surface,
                                icon_surface,
                                content,
                                actions,
                            } => {
                                let Some(window_id) =
                                    source_surface.and_then(|source| {
                                        match source {
                                        core::clipboard::DndSource::Surface(
                                            s,
                                        ) => Some(s),
                                        core::clipboard::DndSource::Widget(
                                            w,
                                        ) => {
                                            // search windows for widget with operation
                                            user_interfaces.iter_mut().find_map(
                                                |(ui_id, ui)| {
                                                    let mut current_operation =
                                            Some(Box::new(OperationWrapper::Id(Box::new(
                                                operation::search_id::search_id(w.clone()),
                                            ))));
                                            let Some(ui_renderer) = window_manager.get_mut(ui_id.clone()).map(|w| &w.renderer) else {
                                                return None;
                                            };
                                        while let Some(mut operation) = current_operation.take()
                                        {
                                            ui
                                                .operate(&ui_renderer, operation.as_mut());
                                            match operation.finish() {
                                                operation::Outcome::None => {
                                                }
                                                operation::Outcome::Some(message) => {
                                                    match message {
                                                        operation::OperationOutputWrapper::Message(_) => {
                                                            unimplemented!();
                                                        }
                                                        operation::OperationOutputWrapper::Id(_) => {
                                                            return Some(ui_id.clone());
                                                        },
                                                    }
                                                }
                                                operation::Outcome::Chain(next) => {
                                                    current_operation = Some(Box::new(OperationWrapper::Wrapper(next)));
                                                }
                                            }
                                        }
                                        None
                                                },
                                            )
                                        },
                                    }
                                    })
                                else {
                                    eprintln!("No source surface");
                                    continue;
                                };

                                let Some(window) =
                                    window_manager.get_mut(window_id)
                                else {
                                    eprintln!("No window");
                                    continue;
                                };

                                let state = &window.state;
                                let icon_surface = icon_surface
                                    .map(|i| {
                                        let i: Box<dyn Any> = i;
                                        i
                                    })
                                    .and_then(|i| {
                                        i.downcast::<Arc<(
                                            core::Element<
                                                'static,
                                                A::Message,
                                                A::Theme,
                                                A::Renderer,
                                            >,
                                            core::widget::tree::State,
                                        )>>(
                                        )
                                        .ok()
                                    })
                                    .map(|e| {
                                        let mut renderer =
                                            compositor.create_renderer();

                                        let e = Arc::into_inner(*e).unwrap();
                                        let (mut e, widget_state) = e;
                                        let lim = core::layout::Limits::new(
                                            Size::new(1., 1.),
                                            Size::new(
                                                state
                                                    .viewport()
                                                    .physical_width()
                                                    as f32,
                                                state
                                                    .viewport()
                                                    .physical_height()
                                                    as f32,
                                            ),
                                        );

                                        let mut tree = core::widget::Tree {
                                            id: e.as_widget().id(),
                                            tag: e.as_widget().tag(),
                                            state: widget_state,
                                            children: e.as_widget().children(),
                                        };

                                        let size = e
                                            .as_widget()
                                            .layout(&mut tree, &renderer, &lim);
                                        e.as_widget_mut().diff(&mut tree);

                                        let size = lim.resolve(
                                            Length::Shrink,
                                            Length::Shrink,
                                            size.size(),
                                        );
                                        let mut surface = compositor
                                            .create_surface(
                                                window.raw.clone(),
                                                size.width.ceil() as u32,
                                                size.height.ceil() as u32,
                                            );
                                        let viewport =
                                            Viewport::with_logical_size(
                                                size,
                                                state.viewport().scale_factor(),
                                            );
                                        let mut ui = UserInterface::build(
                                            e,
                                            size,
                                            user_interface::Cache::default(),
                                            &mut renderer,
                                        );
                                        _ = ui.draw(
                                            &mut renderer,
                                            state.theme(),
                                            &renderer::Style {
                                                icon_color: state.icon_color(),
                                                text_color: state.text_color(),
                                                scale_factor: state
                                                    .scale_factor(),
                                            },
                                            Default::default(),
                                        );
                                        let mut bytes = compositor.screenshot(
                                            &mut renderer,
                                            &mut surface,
                                            &viewport,
                                            core::Color::TRANSPARENT,
                                            &debug.overlay(),
                                        );
                                        for pix in bytes.chunks_exact_mut(4) {
                                            // rgba -> argb little endian
                                            pix.swap(0, 2);
                                        }
                                        Icon::Buffer {
                                            data: Arc::new(bytes),
                                            width: viewport.physical_width(),
                                            height: viewport.physical_height(),
                                            transparent: true,
                                        }
                                    });

                                clipboard.start_dnd_winit(
                                    internal,
                                    DndSurface(Arc::new(Box::new(
                                        window.raw.clone(),
                                    ))),
                                    icon_surface,
                                    content,
                                    actions,
                                );
                            }
                            UserEventWrapper::Dnd(e) => match &e {
                                dnd::DndEvent::Offer(
                                    _,
                                    dnd::OfferEvent::Leave,
                                ) => {
                                    events.push((
                                        cur_dnd_surface,
                                        core::Event::Dnd(e),
                                    ));
                                    cur_dnd_surface = None;
                                }
                                dnd::DndEvent::Offer(
                                    _,
                                    dnd::OfferEvent::Enter { surface, .. },
                                ) => {
                                    let window_handle =
                                        surface.0.window_handle().ok();
                                    let window_id = window_manager
                                        .iter_mut()
                                        .find_map(|(id, window)| {
                                            if window
                                                .raw
                                                .window_handle()
                                                .ok()
                                                .zip(window_handle)
                                                .map(|(a, b)| a == b)
                                                .unwrap_or_default()
                                            {
                                                Some(id)
                                            } else {
                                                None
                                            }
                                        });

                                    cur_dnd_surface = window_id;
                                    events.push((
                                        cur_dnd_surface,
                                        core::Event::Dnd(e),
                                    ));
                                }
                                dnd::DndEvent::Offer(..) => {
                                    events.push((
                                        cur_dnd_surface,
                                        core::Event::Dnd(e),
                                    ));
                                }
                                dnd::DndEvent::Source(_) => {
                                    events.push((None, core::Event::Dnd(e)))
                                }
                            },
                        };
                    }
                    event::Event::WindowEvent {
                        event: window_event,
                        window_id,
                    } => {
                        let Some((id, window)) =
                            window_manager.get_mut_alias(window_id)
                        else {
                            continue;
                        };

                        if matches!(
                            window_event,
                            winit::event::WindowEvent::CloseRequested
                        ) {
                            let w = window_manager.remove(id);
                            let _ = user_interfaces.remove(&id);
                            let _ = ui_caches.remove(&id);
                            if let Some(w) = w.as_ref() {
                                clipboard.register_dnd_destination(
                                    DndSurface(Arc::new(Box::new(
                                        w.raw.clone(),
                                    ))),
                                    Vec::new(),
                                );
                            }

                            events.push((
                                None,
                                core::Event::Window(id, window::Event::Closed),
                            ));

                            if window_manager.is_empty()
                                && w.is_some_and(|w| w.exit_on_close_request)
                            {
                                break 'main;
                            }
                        } else {
                            window.state.update(
                                &window.raw,
                                &window_event,
                                &mut debug,
                            );

                            if let Some(event) = conversion::window_event(
                                id,
                                window_event,
                                window.state.scale_factor(),
                                window.state.modifiers(),
                            ) {
                                events.push((Some(id), event));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = ManuallyDrop::into_inner(user_interfaces);
}

/// Builds a window's [`UserInterface`] for the [`Application`].
fn build_user_interface<'a, A: Application>(
    application: &'a A,
    cache: user_interface::Cache,
    renderer: &mut A::Renderer,
    size: Size,
    debug: &mut Debug,
    id: window::Id,
) -> UserInterface<'a, A::Message, A::Theme, A::Renderer>
where
    A::Theme: DefaultStyle,
{
    debug.view_started();
    let view = application.view(id);
    debug.view_finished();

    debug.layout_started();
    let user_interface = UserInterface::build(view, size, cache, renderer);
    debug.layout_finished();

    user_interface
}

/// Updates a multi-window [`Application`] by feeding it messages, spawning any
/// resulting [`Command`], and tracking its [`Subscription`].
fn update<A: Application + 'static, C, E: Executor + 'static>(
    application: &mut A,
    compositor: &mut C,
    runtime: &mut Runtime<
        E,
        Proxy<UserEventWrapper<A::Message>>,
        UserEventWrapper<A::Message>,
    >,
    clipboard: &mut Clipboard<A::Message>,
    control_sender: &mut mpsc::UnboundedSender<Control>,
    proxy: &mut Proxy<UserEventWrapper<A::Message>>,
    debug: &mut Debug,
    messages: &mut Vec<A::Message>,
    window_manager: &mut WindowManager<A, C>,
    ui_caches: &mut FxHashMap<window::Id, user_interface::Cache>,
) where
    C: Compositor<Renderer = A::Renderer> + 'static,
    A::Theme: DefaultStyle,
    A::Message: Send + 'static,
{
    for message in messages.drain(..) {
        debug.log_message(&message);
        debug.update_started();

        let command = runtime.enter(|| application.update(message));
        debug.update_finished();

        run_command(
            application,
            compositor,
            command,
            runtime,
            clipboard,
            control_sender,
            proxy,
            debug,
            window_manager,
            ui_caches,
        );
    }

    let subscription = application
        .subscription()
        .map(subscription_map::<A, E>)
        .into_recipes();
    runtime.track(subscription);
}

/// Runs the actions of a [`Command`].
fn run_command<A, C, E>(
    application: &A,
    compositor: &mut C,
    command: Command<A::Message>,
    runtime: &mut Runtime<
        E,
        Proxy<UserEventWrapper<A::Message>>,
        UserEventWrapper<A::Message>,
    >,
    clipboard: &mut Clipboard<A::Message>,
    control_sender: &mut mpsc::UnboundedSender<Control>,
    proxy: &mut Proxy<UserEventWrapper<A::Message>>,
    debug: &mut Debug,
    window_manager: &mut WindowManager<A, C>,
    ui_caches: &mut FxHashMap<window::Id, user_interface::Cache>,
) where
    A: Application,
    E: Executor,
    C: Compositor<Renderer = A::Renderer> + 'static,
    A::Theme: DefaultStyle,
    A::Message: Send + 'static,
{
    use crate::runtime::clipboard;
    use crate::runtime::system;
    use crate::runtime::window;

    for action in command.actions() {
        match action {
            command::Action::Future(future) => {
                runtime.spawn(Box::pin(future.map(UserEventWrapper::Message)));
            }
            command::Action::Stream(stream) => {
                runtime.run(Box::pin(stream.map(UserEventWrapper::Message)));
            }
            command::Action::Clipboard(action) => match action {
                clipboard::Action::Read(tag, kind) => {
                    let message = tag(clipboard.read(kind));

                    proxy.send(UserEventWrapper::Message(message));
                }
                clipboard::Action::Write(contents, kind) => {
                    clipboard.write(kind, contents);
                }
                clipboard::Action::WriteData(contents, kind) => {
                    clipboard.write_data(kind, ClipboardStoreData(contents))
                }
                clipboard::Action::ReadData(allowed, to_msg, kind) => {
                    let contents = clipboard.read_data(kind, allowed);
                    let message = to_msg(contents);
                    _ = proxy.send_event(UserEventWrapper::Message(message));
                }
            },
            command::Action::Window(action) => match action {
                window::Action::Spawn(id, settings) => {
                    let monitor = window_manager.last_monitor();

                    control_sender
                        .start_send(Control::CreateWindow {
                            id,
                            settings,
                            title: application.title(id),
                            monitor,
                        })
                        .expect("Send control action");
                }
                window::Action::Close(id) => {
                    let w = window_manager.remove(id);
                    let _ = ui_caches.remove(&id);
                    if let Some(w) = w.as_ref() {
                        clipboard.register_dnd_destination(
                            DndSurface(Arc::new(Box::new(w.raw.clone()))),
                            Vec::new(),
                        );
                    }

                    if window_manager.is_empty()
                        && w.is_some_and(|w| w.exit_on_close_request)
                    {
                        control_sender
                            .start_send(Control::Exit)
                            .expect("Send control action");
                    }
                }
                window::Action::Drag(id) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let _ = window.raw.drag_window();
                    }
                }
                window::Action::Resize(id, size) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let _ = window.raw.request_inner_size(
                            winit::dpi::LogicalSize {
                                width: size.width,
                                height: size.height,
                            },
                        );
                    }
                }
                window::Action::FetchSize(id, callback) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let size = window
                            .raw
                            .inner_size()
                            .to_logical(window.raw.scale_factor());

                        proxy.send(UserEventWrapper::Message(callback(
                            Size::new(size.width, size.height),
                        )));
                    }
                }
                window::Action::FetchMaximized(id, callback) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        proxy.send(UserEventWrapper::Message(callback(
                            window.raw.is_maximized(),
                        )));
                    }
                }
                window::Action::Maximize(id, maximized) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_maximized(maximized);
                    }
                }
                window::Action::FetchMinimized(id, callback) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        proxy.send(UserEventWrapper::Message(callback(
                            window.raw.is_minimized(),
                        )));
                    }
                }
                window::Action::Minimize(id, minimized) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_minimized(minimized);
                    }
                }
                window::Action::FetchPosition(id, callback) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let position = window
                            .raw
                            .inner_position()
                            .map(|position| {
                                let position = position.to_logical::<f32>(
                                    window.raw.scale_factor(),
                                );

                                Point::new(position.x, position.y)
                            })
                            .ok();

                        proxy.send(UserEventWrapper::Message(callback(
                            position,
                        )));
                    }
                }
                window::Action::Move(id, position) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_outer_position(
                            winit::dpi::LogicalPosition {
                                x: position.x,
                                y: position.y,
                            },
                        );
                    }
                }
                window::Action::ChangeMode(id, mode) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_visible(conversion::visible(mode));
                        window.raw.set_fullscreen(conversion::fullscreen(
                            window.raw.current_monitor(),
                            mode,
                        ));
                    }
                }
                window::Action::ChangeIcon(id, icon) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_window_icon(conversion::icon(icon));
                    }
                }
                window::Action::FetchMode(id, tag) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let mode = if window.raw.is_visible().unwrap_or(true) {
                            conversion::mode(window.raw.fullscreen())
                        } else {
                            core::window::Mode::Hidden
                        };

                        proxy.send(UserEventWrapper::Message(tag(mode)));
                    }
                }
                window::Action::ToggleMaximize(id) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_maximized(!window.raw.is_maximized());
                    }
                }
                window::Action::ToggleDecorations(id) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.set_decorations(!window.raw.is_decorated());
                    }
                }
                window::Action::RequestUserAttention(id, attention_type) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.request_user_attention(
                            attention_type.map(conversion::user_attention),
                        );
                    }
                }
                window::Action::GainFocus(id) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window.raw.focus_window();
                    }
                }
                window::Action::ChangeLevel(id, level) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        window
                            .raw
                            .set_window_level(conversion::window_level(level));
                    }
                }
                window::Action::ShowSystemMenu(id) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        if let mouse::Cursor::Available(point) =
                            window.state.cursor()
                        {
                            window.raw.show_window_menu(
                                winit::dpi::LogicalPosition {
                                    x: point.x,
                                    y: point.y,
                                },
                            );
                        }
                    }
                }
                window::Action::FetchId(id, tag) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        proxy.send(UserEventWrapper::Message(tag(window
                            .raw
                            .id()
                            .into())));
                    }
                }
                window::Action::RunWithHandle(id, tag) => {
                    use window::raw_window_handle::HasWindowHandle;

                    if let Some(handle) = window_manager
                        .get_mut(id)
                        .and_then(|window| window.raw.window_handle().ok())
                    {
                        proxy.send(UserEventWrapper::Message(tag(handle)));
                    }
                }
                window::Action::Screenshot(id, tag) => {
                    if let Some(window) = window_manager.get_mut(id) {
                        let bytes = compositor.screenshot(
                            &mut window.renderer,
                            &mut window.surface,
                            window.state.viewport(),
                            window.state.background_color(),
                            &debug.overlay(),
                        );

                        proxy.send(UserEventWrapper::Message(tag(
                            window::Screenshot::new(
                                bytes,
                                window.state.physical_size(),
                                window.state.viewport().scale_factor(),
                            ),
                        )));
                    }
                }
            },
            command::Action::System(action) => match action {
                system::Action::QueryInformation(_tag) => {
                    #[cfg(feature = "system")]
                    {
                        let graphics_info = compositor.fetch_information();
                        let mut proxy = proxy.clone();

                        let _ = std::thread::spawn(move || {
                            let information =
                                crate::system::information(graphics_info);

                            let message = _tag(information);

                            proxy.send(UserEventWrapper::Message(message));
                        });
                    }
                }
            },
            command::Action::Widget(action) => {
                let mut current_operation =
                    Some(Box::new(OperationWrapper::Message(action)));
                let mut uis = build_user_interfaces(
                    application,
                    debug,
                    window_manager,
                    std::mem::take(ui_caches),
                    clipboard,
                );

                while let Some(mut operation) = current_operation.take() {
                    for (id, ui) in uis.iter_mut() {
                        if let Some(window) = window_manager.get_mut(*id) {
                            ui.operate(&window.renderer, operation.as_mut());

                            match operation.finish() {
                                operation::Outcome::None => {}
                                operation::Outcome::Some(message) => {
                                    match message {
                                operation::OperationOutputWrapper::Message(
                                    m,
                                ) => {
                                    proxy
                                        .send(
                                            UserEventWrapper::Message(m),
                                        );
                                }
                                operation::OperationOutputWrapper::Id(_) => {
                                    // TODO ASHLEY should not ever happen, should this panic!()?
                                }
                            }
                                }
                                operation::Outcome::Chain(next) => {
                                    current_operation = Some(Box::new(
                                        OperationWrapper::Wrapper(next),
                                    ));
                                }
                            }
                        }
                    }
                }

                *ui_caches =
                    uis.drain().map(|(id, ui)| (id, ui.into_cache())).collect();
            }
            command::Action::LoadFont { bytes, tagger } => {
                // TODO: Error handling (?)
                compositor.load_font(bytes.clone());

                proxy.send(UserEventWrapper::Message(tagger(Ok(()))));
            }
            command::Action::Custom(_) => {
                log::warn!("Unsupported custom action in `iced_winit` shell");
            }
            command::Action::PlatformSpecific(_) => {
                tracing::warn!("Platform specific commands are not supported yet in multi-window winit mode.");
            }
            command::Action::Dnd(a) => match a {
                iced_runtime::dnd::DndAction::RegisterDndDestination {
                    surface,
                    rectangles,
                } => {
                    clipboard.register_dnd_destination(surface, rectangles);
                }
                iced_runtime::dnd::DndAction::StartDnd {
                    internal,
                    source_surface,
                    icon_surface,
                    content,
                    actions,
                } => clipboard.start_dnd(
                    internal,
                    source_surface,
                    icon_surface,
                    content,
                    actions,
                ),
                iced_runtime::dnd::DndAction::EndDnd => {
                    clipboard.end_dnd();
                }
                iced_runtime::dnd::DndAction::PeekDnd(m, to_msg) => {
                    let data = clipboard.peek_dnd(m);
                    let message = to_msg(data);
                    proxy
                        .send_event(UserEventWrapper::Message(message))
                        .expect("Send message to event loop");
                }
                iced_runtime::dnd::DndAction::SetAction(a) => {
                    clipboard.set_action(a);
                }
            },
        }
    }
}

/// Build the user interface for every window.
pub fn build_user_interfaces<'a, A: Application, C>(
    application: &'a A,
    debug: &mut Debug,
    window_manager: &mut WindowManager<A, C>,
    mut cached_user_interfaces: FxHashMap<window::Id, user_interface::Cache>,
    clipboard: &mut Clipboard<A::Message>,
) -> FxHashMap<window::Id, UserInterface<'a, A::Message, A::Theme, A::Renderer>>
where
    C: Compositor<Renderer = A::Renderer>,
    A::Theme: DefaultStyle,
{
    cached_user_interfaces
        .drain()
        .filter_map(|(id, cache)| {
            let window = window_manager.get_mut(id)?;
            let interface = build_user_interface(
                application,
                cache,
                &mut window.renderer,
                window.state.logical_size(),
                debug,
                id,
            );

            let dnd_rectangles = interface.dnd_rectangles(
                window.prev_dnd_destination_rectangles_count,
                &window.renderer,
            );
            let new_dnd_rectangles_count = dnd_rectangles.as_ref().len();
            if new_dnd_rectangles_count > 0
                || window.prev_dnd_destination_rectangles_count > 0
            {
                clipboard.register_dnd_destination(
                    DndSurface(Arc::new(Box::new(window.raw.clone()))),
                    dnd_rectangles.into_rectangles(),
                );
            }

            window.prev_dnd_destination_rectangles_count =
                new_dnd_rectangles_count;

            Some((id, interface))
        })
        .collect()
}

/// Returns true if the provided event should cause an [`Application`] to
/// exit.
pub fn user_force_quit(
    event: &winit::event::WindowEvent,
    _modifiers: winit::keyboard::ModifiersState,
) -> bool {
    match event {
        #[cfg(target_os = "macos")]
        winit::event::WindowEvent::KeyboardInput {
            event:
                winit::event::KeyEvent {
                    logical_key: winit::keyboard::Key::Character(c),
                    state: winit::event::ElementState::Pressed,
                    ..
                },
            ..
        } if c == "q" && _modifiers.super_key() => true,
        _ => false,
    }
}
