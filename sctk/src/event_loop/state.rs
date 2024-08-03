use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    num::NonZeroU32,
};

use crate::{
    application::Event,
    dpi::LogicalSize,
    handlers::{
        wp_fractional_scaling::FractionalScalingManager,
        wp_viewporter::ViewporterState,
    },
    sctk_event::{
        LayerSurfaceEventVariant, PopupEventVariant, SctkEvent,
        WindowEventVariant,
    },
};

use iced_runtime::{
    command::platform_specific::{
        self,
        wayland::{
            data_device::DataFromMimeType,
            layer_surface::{IcedMargin, IcedOutput, SctkLayerSurfaceSettings},
            popup::SctkPopupSettings,
            window::SctkWindowSettings,
        },
    },
    core::{touch, Point},
    keyboard::Modifiers,
    window,
};
use sctk::{
    activation::ActivationState,
    compositor::CompositorState,
    data_device_manager::{
        data_device::DataDevice, data_offer::DragOffer,
        data_source::DragSource, DataDeviceManagerState, WritePipe,
    },
    error::GlobalError,
    output::OutputState,
    reexports::{
        calloop::{LoopHandle, RegistrationToken},
        client::{
            delegate_noop,
            protocol::{
                wl_keyboard::WlKeyboard,
                wl_output::WlOutput,
                wl_region::WlRegion,
                wl_seat::WlSeat,
                wl_subsurface::WlSubsurface,
                wl_surface::{self, WlSurface},
                wl_touch::WlTouch,
            },
            Connection, QueueHandle,
        },
    },
    registry::RegistryState,
    seat::{
        keyboard::KeyEvent,
        pointer::{CursorIcon, ThemedPointer},
        SeatState,
    },
    session_lock::{
        SessionLock, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            popup::{Popup, PopupConfigure},
            window::{Window, WindowConfigure, WindowDecorations},
            XdgPositioner, XdgShell, XdgSurface,
        },
        WaylandSurface,
    },
    shm::{multi::MultiPool, Shm},
};
use wayland_protocols::wp::{
    fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
    viewporter::client::wp_viewport::WpViewport,
};

#[derive(Debug)]
pub(crate) struct SctkSeat {
    pub(crate) seat: WlSeat,
    pub(crate) kbd: Option<WlKeyboard>,
    pub(crate) kbd_focus: Option<WlSurface>,
    pub(crate) last_kbd_press: Option<(KeyEvent, u32)>,
    pub(crate) ptr: Option<ThemedPointer>,
    pub(crate) ptr_focus: Option<WlSurface>,
    pub(crate) last_ptr_press: Option<(u32, u32, u32)>, // (time, button, serial)
    pub(crate) touch: Option<WlTouch>,
    pub(crate) last_touch_down: Option<(u32, i32, u32)>, // (time, point, serial)
    pub(crate) _modifiers: Modifiers,
    pub(crate) data_device: DataDevice,
    // Cursor icon currently set (by CSDs, or application)
    pub(crate) active_icon: Option<CursorIcon>,
    // Cursor icon set by application
    pub(crate) icon: Option<CursorIcon>,
}

impl SctkSeat {
    pub(crate) fn set_cursor(&mut self, conn: &Connection, icon: CursorIcon) {
        if let Some(ptr) = self.ptr.as_ref() {
            ptr.set_cursor(conn, icon);
            self.active_icon = Some(icon);
        }
    }
}

#[derive(Debug, Clone)]
pub struct SctkWindow<T> {
    pub(crate) id: window::Id,
    pub(crate) window: Window,
    pub(crate) scale_factor: Option<f64>,
    pub(crate) requested_size: Option<(NonZeroU32, NonZeroU32)>,
    pub(crate) current_size: (NonZeroU32, NonZeroU32),
    pub(crate) last_configure: Option<WindowConfigure>,
    pub(crate) resizable: Option<f64>,
    /// Requests that SCTK window should perform.
    pub(crate) _pending_requests:
        Vec<platform_specific::wayland::window::Action<T>>,
    pub(crate) wp_fractional_scale: Option<WpFractionalScaleV1>,
    pub(crate) wp_viewport: Option<WpViewport>,
}

impl<T> SctkWindow<T> {
    pub(crate) fn set_size(&mut self, logical_size: LogicalSize<NonZeroU32>) {
        self.requested_size = Some((logical_size.width, logical_size.height));
        self.update_size((Some(logical_size.width), Some(logical_size.height)))
    }

    pub(crate) fn update_size(
        &mut self,
        (width, height): (Option<NonZeroU32>, Option<NonZeroU32>),
    ) {
        let (width, height) = (
            width.unwrap_or_else(|| self.current_size.0),
            height.unwrap_or_else(|| self.current_size.1),
        );
        if self.current_size == (width, height) {
            return;
        }
        self.window
            .set_window_geometry(0, 0, width.get(), height.get());
        self.current_size = (width, height);
        // Update the target viewport, this is used if and only if fractional scaling is in use.
        if let Some(viewport) = self.wp_viewport.as_ref() {
            // Set inner size without the borders.
            viewport.set_destination(width.get() as _, height.get() as _);
        }
    }
}

#[derive(Debug, Clone)]
pub struct SctkLayerSurface<T> {
    pub(crate) id: window::Id,
    pub(crate) surface: LayerSurface,
    pub(crate) requested_size: (Option<u32>, Option<u32>),
    pub(crate) current_size: Option<LogicalSize<u32>>,
    pub(crate) layer: Layer,
    pub(crate) anchor: Anchor,
    pub(crate) keyboard_interactivity: KeyboardInteractivity,
    pub(crate) margin: IcedMargin,
    pub(crate) exclusive_zone: i32,
    pub(crate) last_configure: Option<LayerSurfaceConfigure>,
    pub(crate) _pending_requests:
        Vec<platform_specific::wayland::layer_surface::Action<T>>,
    pub(crate) scale_factor: Option<f64>,
    pub(crate) wp_fractional_scale: Option<WpFractionalScaleV1>,
    pub(crate) wp_viewport: Option<WpViewport>,
}

impl<T> SctkLayerSurface<T> {
    pub(crate) fn set_size(&mut self, w: Option<u32>, h: Option<u32>) {
        self.requested_size = (w, h);

        let (w, h) = (w.unwrap_or_default(), h.unwrap_or_default());
        self.surface.set_size(w, h);
    }

    pub(crate) fn update_viewport(&mut self, w: u32, h: u32) {
        self.current_size = Some(LogicalSize::new(w, h));
        if let Some(viewport) = self.wp_viewport.as_ref() {
            // Set inner size without the borders.
            viewport.set_destination(w as i32, h as i32);
        }
    }
}

#[derive(Debug, Clone)]
pub enum SctkSurface {
    LayerSurface(WlSurface),
    Window(WlSurface),
    Popup(WlSurface),
}

impl SctkSurface {
    pub fn wl_surface(&self) -> &WlSurface {
        match self {
            SctkSurface::LayerSurface(s)
            | SctkSurface::Window(s)
            | SctkSurface::Popup(s) => s,
        }
    }
}

#[derive(Debug)]
pub struct SctkPopup<T> {
    pub(crate) popup: Popup,
    pub(crate) last_configure: Option<PopupConfigure>,
    // pub(crate) positioner: XdgPositioner,
    pub(crate) _pending_requests:
        Vec<platform_specific::wayland::popup::Action<T>>,
    pub(crate) data: SctkPopupData,
    pub(crate) scale_factor: Option<f64>,
    pub(crate) wp_fractional_scale: Option<WpFractionalScaleV1>,
    pub(crate) wp_viewport: Option<WpViewport>,
}

impl<T> SctkPopup<T> {
    pub(crate) fn set_size(&mut self, w: u32, h: u32, token: u32) {
        // update geometry
        self.popup
            .xdg_surface()
            .set_window_geometry(0, 0, w as i32, h as i32);
        // update positioner
        self.data.positioner.set_size(w as i32, h as i32);
        self.popup.reposition(&self.data.positioner, token);
    }
}

#[derive(Debug)]
pub struct SctkLockSurface {
    pub(crate) id: window::Id,
    pub(crate) session_lock_surface: SessionLockSurface,
    pub(crate) last_configure: Option<SessionLockSurfaceConfigure>,
}

pub struct Dnd<T> {
    pub(crate) origin_id: window::Id,
    pub(crate) origin: WlSurface,
    pub(crate) source: Option<(DragSource, Box<dyn DataFromMimeType>)>,
    pub(crate) icon_surface: Option<(WlSurface, window::Id)>,
    pub(crate) pending_requests:
        Vec<platform_specific::wayland::data_device::Action<T>>,
    pub(crate) pipe: Option<WritePipe>,
    pub(crate) cur_write: Option<(Vec<u8>, usize, RegistrationToken)>,
}

impl<T> Debug for Dnd<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Dnd")
            .field(&self.origin_id)
            .field(&self.origin)
            .field(&self.icon_surface)
            .field(&self.pending_requests)
            .field(&self.pipe)
            .field(&self.cur_write)
            .finish()
    }
}

#[derive(Debug)]
pub struct SctkDragOffer {
    pub(crate) dropped: bool,
    pub(crate) offer: Option<DragOffer>,
    pub(crate) cur_read: Option<(String, Vec<u8>, RegistrationToken)>,
    pub(crate) surface: WlSurface,
}

#[derive(Debug)]
pub struct SctkPopupData {
    pub(crate) id: window::Id,
    pub(crate) parent: SctkSurface,
    pub(crate) toplevel: WlSurface,
    pub(crate) positioner: XdgPositioner,
}

/// Wrapper to carry sctk state.
pub struct SctkState<T> {
    pub(crate) connection: Connection,

    /// the cursor wl_surface
    pub(crate) _cursor_surface: Option<wl_surface::WlSurface>,
    /// a memory pool
    pub(crate) _multipool: Option<MultiPool<WlSurface>>,

    // all present outputs
    pub(crate) outputs: Vec<WlOutput>,
    // though (for now) only one seat will be active in an iced application at a time, all ought to be tracked
    // Active seat is the first seat in the list
    pub(crate) seats: Vec<SctkSeat>,
    // Windows / Surfaces
    /// Window list containing all SCTK windows. Since those windows aren't allowed
    /// to be sent to other threads, they live on the event loop's thread
    /// and requests from winit's windows are being forwarded to them either via
    /// `WindowUpdate` or buffer on the associated with it `WindowHandle`.
    pub(crate) windows: Vec<SctkWindow<T>>,
    pub(crate) layer_surfaces: Vec<SctkLayerSurface<T>>,
    pub(crate) popups: Vec<SctkPopup<T>>,
    pub(crate) lock_surfaces: Vec<SctkLockSurface>,
    pub(crate) dnd_source: Option<Dnd<T>>,
    pub(crate) _kbd_focus: Option<WlSurface>,
    pub(crate) touch_points: HashMap<touch::Finger, (WlSurface, Point)>,

    /// Window updates, which are coming from SCTK or the compositor, which require
    /// calling back to the sctk's downstream. They are handled right in the event loop,
    /// unlike the ones coming from buffers on the `WindowHandle`'s.
    pub compositor_updates: Vec<SctkEvent>,

    /// data data_device
    pub(crate) dnd_offer: Option<SctkDragOffer>,
    pub(crate) _accept_counter: u32,
    /// A sink for window and device events that is being filled during dispatching
    /// event loop and forwarded downstream afterwards.
    pub(crate) sctk_events: Vec<SctkEvent>,
    pub(crate) frame_events: Vec<(WlSurface, u32)>,

    /// pending user events
    pub pending_user_events: Vec<Event<T>>,

    // handles
    pub(crate) queue_handle: QueueHandle<Self>,
    pub(crate) loop_handle: LoopHandle<'static, Self>,

    // sctk state objects
    /// Viewporter state on the given window.
    pub viewporter_state: Option<ViewporterState<T>>,
    pub(crate) fractional_scaling_manager: Option<FractionalScalingManager<T>>,
    pub(crate) registry_state: RegistryState,
    pub(crate) seat_state: SeatState,
    pub(crate) output_state: OutputState,
    pub(crate) compositor_state: CompositorState,
    pub(crate) shm_state: Shm,
    pub(crate) xdg_shell_state: XdgShell,
    pub(crate) layer_shell: Option<LayerShell>,
    pub(crate) data_device_manager_state: DataDeviceManagerState,
    pub(crate) activation_state: Option<ActivationState>,
    pub(crate) session_lock_state: SessionLockState,
    pub(crate) session_lock: Option<SessionLock>,
    pub(crate) token_ctr: u32,
}

/// An error that occurred while running an application.
#[derive(Debug, thiserror::Error)]
pub enum PopupCreationError {
    /// Positioner creation failed
    #[error("Positioner creation failed")]
    PositionerCreationFailed(GlobalError),

    /// The specified parent is missing
    #[error("The specified parent is missing")]
    ParentMissing,

    /// The specified size is missing
    #[error("The specified size is missing")]
    SizeMissing,

    /// Popup creation failed
    #[error("Popup creation failed")]
    PopupCreationFailed(GlobalError),
}

/// An error that occurred while running an application.
#[derive(Debug, thiserror::Error)]
pub enum LayerSurfaceCreationError {
    /// Layer shell is not supported by the compositor
    #[error("Layer shell is not supported by the compositor")]
    LayerShellNotSupported,

    /// WlSurface creation failed
    #[error("WlSurface creation failed")]
    WlSurfaceCreationFailed(GlobalError),

    /// LayerSurface creation failed
    #[error("Layer Surface creation failed")]
    LayerSurfaceCreationFailed(GlobalError),
}

/// An error that occurred while starting a drag and drop operation.
#[derive(Debug, thiserror::Error)]
pub enum DndStartError {}

impl<T> SctkState<T> {
    pub fn scale_factor_changed(
        &mut self,
        surface: &WlSurface,
        scale_factor: f64,
        legacy: bool,
    ) {
        if let Some(window) = self
            .windows
            .iter_mut()
            .find(|w| w.window.wl_surface() == surface)
        {
            if legacy && window.wp_fractional_scale.is_some() {
                return;
            }
            window.scale_factor = Some(scale_factor);
            if legacy {
                let _ = window.window.set_buffer_scale(scale_factor as u32);
            }
            self.compositor_updates.push(SctkEvent::WindowEvent {
                variant: WindowEventVariant::ScaleFactorChanged(
                    scale_factor,
                    window.wp_viewport.clone(),
                ),
                id: window.window.wl_surface().clone(),
            });
        }

        if let Some(popup) = self
            .popups
            .iter_mut()
            .find(|p| p.popup.wl_surface() == surface)
        {
            if legacy && popup.wp_fractional_scale.is_some() {
                return;
            }
            popup.scale_factor = Some(scale_factor);
            if legacy {
                popup.popup.wl_surface().set_buffer_scale(scale_factor as _);
            }
            self.compositor_updates.push(SctkEvent::PopupEvent {
                variant: PopupEventVariant::ScaleFactorChanged(
                    scale_factor,
                    popup.wp_viewport.clone(),
                ),
                id: popup.popup.wl_surface().clone(),
                toplevel_id: popup.data.toplevel.clone(),
                parent_id: popup.data.parent.wl_surface().clone(),
            });
        }

        if let Some(layer_surface) = self
            .layer_surfaces
            .iter_mut()
            .find(|l| l.surface.wl_surface() == surface)
        {
            if legacy && layer_surface.wp_fractional_scale.is_some() {
                return;
            }
            layer_surface.scale_factor = Some(scale_factor);
            if legacy {
                let _ =
                    layer_surface.surface.set_buffer_scale(scale_factor as u32);
            }
            self.compositor_updates.push(SctkEvent::LayerSurfaceEvent {
                variant: LayerSurfaceEventVariant::ScaleFactorChanged(
                    scale_factor,
                    layer_surface.wp_viewport.clone(),
                ),
                id: layer_surface.surface.wl_surface().clone(),
            });
        }

        // TODO winit sets cursor size after handling the change for the window, so maybe that should be done as well.
    }
}

impl<T> SctkState<T>
where
    T: 'static + Debug,
{
    pub fn get_popup(
        &mut self,
        settings: SctkPopupSettings,
    ) -> Result<(window::Id, WlSurface, WlSurface, WlSurface), PopupCreationError>
    {
        let (parent, toplevel) = if let Some(parent) =
            self.layer_surfaces.iter().find(|l| l.id == settings.parent)
        {
            (
                SctkSurface::LayerSurface(parent.surface.wl_surface().clone()),
                parent.surface.wl_surface().clone(),
            )
        } else if let Some(parent) =
            self.windows.iter().find(|w| w.id == settings.parent)
        {
            (
                SctkSurface::Window(parent.window.wl_surface().clone()),
                parent.window.wl_surface().clone(),
            )
        } else if let Some(i) = self
            .popups
            .iter()
            .position(|p| p.data.id == settings.parent)
        {
            let parent = &self.popups[i];
            (
                SctkSurface::Popup(parent.popup.wl_surface().clone()),
                parent.data.toplevel.clone(),
            )
        } else {
            return Err(PopupCreationError::ParentMissing);
        };

        let size = if settings.positioner.size.is_none() {
            return Err(PopupCreationError::SizeMissing);
        } else {
            settings.positioner.size.unwrap()
        };

        let positioner = XdgPositioner::new(&self.xdg_shell_state)
            .map_err(PopupCreationError::PositionerCreationFailed)?;
        positioner.set_anchor(settings.positioner.anchor);
        positioner.set_anchor_rect(
            settings.positioner.anchor_rect.x,
            settings.positioner.anchor_rect.y,
            settings.positioner.anchor_rect.width,
            settings.positioner.anchor_rect.height,
        );
        if let Ok(constraint_adjustment) =
            settings.positioner.constraint_adjustment.try_into()
        {
            positioner.set_constraint_adjustment(constraint_adjustment);
        }
        positioner.set_gravity(settings.positioner.gravity);
        positioner.set_offset(
            settings.positioner.offset.0,
            settings.positioner.offset.1,
        );
        if settings.positioner.reactive {
            positioner.set_reactive();
        }
        positioner.set_size(size.0 as i32, size.1 as i32);

        let grab = settings.grab;

        let wl_surface =
            self.compositor_state.create_surface(&self.queue_handle);

        let (toplevel, popup) = match &parent {
            SctkSurface::LayerSurface(parent) => {
                let Some(parent_layer_surface) = self
                    .layer_surfaces
                    .iter()
                    .find(|w| w.surface.wl_surface() == parent)
                else {
                    return Err(PopupCreationError::ParentMissing);
                };
                let popup = Popup::from_surface(
                    None,
                    &positioner,
                    &self.queue_handle,
                    wl_surface.clone(),
                    &self.xdg_shell_state,
                )
                .map_err(PopupCreationError::PopupCreationFailed)?;
                parent_layer_surface.surface.get_popup(popup.xdg_popup());
                (parent_layer_surface.surface.wl_surface(), popup)
            }
            SctkSurface::Window(parent) => {
                let Some(parent_window) = self
                    .windows
                    .iter()
                    .find(|w| w.window.wl_surface() == parent)
                else {
                    return Err(PopupCreationError::ParentMissing);
                };
                (
                    parent_window.window.wl_surface(),
                    Popup::from_surface(
                        Some(parent_window.window.xdg_surface()),
                        &positioner,
                        &self.queue_handle,
                        wl_surface.clone(),
                        &self.xdg_shell_state,
                    )
                    .map_err(PopupCreationError::PopupCreationFailed)?,
                )
            }
            SctkSurface::Popup(parent) => {
                let Some(parent_xdg) = self.popups.iter().find_map(|p| {
                    (p.popup.wl_surface() == parent)
                        .then(|| p.popup.xdg_surface())
                }) else {
                    return Err(PopupCreationError::ParentMissing);
                };

                (
                    &toplevel,
                    Popup::from_surface(
                        Some(parent_xdg),
                        &positioner,
                        &self.queue_handle,
                        wl_surface.clone(),
                        &self.xdg_shell_state,
                    )
                    .map_err(PopupCreationError::PopupCreationFailed)?,
                )
            }
        };
        if grab {
            if let Some(s) = self.seats.first() {
                popup.xdg_popup().grab(
                    &s.seat,
                    s.last_ptr_press.map(|p| p.2).unwrap_or_else(|| {
                        s.last_kbd_press
                            .as_ref()
                            .map(|p| p.1)
                            .unwrap_or_default()
                    }),
                )
            }
        }
        wl_surface.commit();

        let wp_viewport = self.viewporter_state.as_ref().map(|state| {
            let viewport =
                state.get_viewport(popup.wl_surface(), &self.queue_handle);
            viewport.set_destination(size.0 as i32, size.1 as i32);
            viewport
        });
        let wp_fractional_scale =
            self.fractional_scaling_manager.as_ref().map(|fsm| {
                fsm.fractional_scaling(popup.wl_surface(), &self.queue_handle)
            });

        self.popups.push(SctkPopup {
            popup: popup.clone(),
            data: SctkPopupData {
                id: settings.id,
                parent: parent.clone(),
                toplevel: toplevel.clone(),
                positioner,
            },
            last_configure: None,
            _pending_requests: Default::default(),
            wp_viewport,
            wp_fractional_scale,
            scale_factor: None,
        });

        Ok((
            settings.id,
            parent.wl_surface().clone(),
            toplevel.clone(),
            popup.wl_surface().clone(),
        ))
    }

    pub fn get_window(
        &mut self,
        settings: SctkWindowSettings,
    ) -> (window::Id, WlSurface) {
        let SctkWindowSettings {
            size,
            client_decorations,

            window_id,
            app_id,
            title,

            size_limits,
            resizable,
            xdg_activation_token,
            ..
        } = settings;
        // TODO Ashley: set window as opaque if transparency is false
        // TODO Ashley: set icon
        // TODO Ashley: save settings for window
        // TODO Ashley: decorations
        let wl_surface =
            self.compositor_state.create_surface(&self.queue_handle);
        let decorations: WindowDecorations = if client_decorations {
            WindowDecorations::RequestClient
        } else {
            WindowDecorations::RequestServer
        };
        let window = self.xdg_shell_state.create_window(
            wl_surface.clone(),
            decorations,
            &self.queue_handle,
        );
        if let Some(app_id) = app_id {
            window.set_app_id(app_id);
        }
        // TODO better way of handling size limits
        let min_size = size_limits.min();
        let min_size = if min_size.width as i32 <= 0
            || min_size.height as i32 <= 0
            || min_size.width > u16::MAX as f32
            || min_size.height > u16::MAX as f32
        {
            None
        } else {
            Some((min_size.width as u32, min_size.height as u32))
        };
        let max_size: iced_futures::core::Size = size_limits.max();
        let max_size = if max_size.width as i32 <= 0
            || max_size.height as i32 <= 0
            || max_size.width > u16::MAX as f32
            || max_size.height > u16::MAX as f32
        {
            None
        } else {
            Some((max_size.width as u32, max_size.height as u32))
        };
        if min_size.is_some() {
            window.set_min_size(min_size);
        }
        if max_size.is_some() {
            window.set_max_size(max_size);
        }

        if let Some(title) = title {
            window.set_title(title);
        }
        // if let Some(parent) = parent.and_then(|p| self.windows.iter().find(|w| w.window.wl_surface().id() == p)) {
        //     window.set_parent(Some(&parent.window));
        // }
        window.xdg_surface().set_window_geometry(
            0,
            0,
            size.0 as i32,
            size.1 as i32,
        );

        window.commit();

        if let (Some(token), Some(activation_state)) =
            (xdg_activation_token, self.activation_state.as_ref())
        {
            activation_state.activate::<()>(window.wl_surface(), token);
        }

        let wp_viewport = self.viewporter_state.as_ref().map(|state| {
            state.get_viewport(window.wl_surface(), &self.queue_handle)
        });
        let wp_fractional_scale =
            self.fractional_scaling_manager.as_ref().map(|fsm| {
                fsm.fractional_scaling(window.wl_surface(), &self.queue_handle)
            });

        let w = NonZeroU32::new(size.0 as u32)
            .unwrap_or_else(|| NonZeroU32::new(1).unwrap());
        let h = NonZeroU32::new(size.1 as u32)
            .unwrap_or_else(|| NonZeroU32::new(1).unwrap());
        self.windows.push(SctkWindow {
            id: window_id,
            window,
            scale_factor: None,
            requested_size: Some((w, h)),
            current_size: (w, h),
            last_configure: None,
            _pending_requests: Vec::new(),
            resizable,
            wp_viewport,
            wp_fractional_scale,
        });
        (window_id, wl_surface)
    }

    pub fn get_layer_surface(
        &mut self,
        SctkLayerSurfaceSettings {
            id,
            layer,
            keyboard_interactivity,
            pointer_interactivity,
            anchor,
            output,
            namespace,
            margin,
            size,
            exclusive_zone,
            ..
        }: SctkLayerSurfaceSettings,
    ) -> Result<(window::Id, WlSurface), LayerSurfaceCreationError> {
        let wl_output = match output {
            IcedOutput::All => None, // TODO
            IcedOutput::Active => None,
            IcedOutput::Output(output) => Some(output),
        };

        let layer_shell = self
            .layer_shell
            .as_ref()
            .ok_or(LayerSurfaceCreationError::LayerShellNotSupported)?;
        let wl_surface =
            self.compositor_state.create_surface(&self.queue_handle);
        let mut size = size.unwrap();
        if anchor.contains(Anchor::BOTTOM.union(Anchor::TOP)) {
            size.1 = None;
        }
        if anchor.contains(Anchor::LEFT.union(Anchor::RIGHT)) {
            size.0 = None;
        }
        let layer_surface = layer_shell.create_layer_surface(
            &self.queue_handle,
            wl_surface.clone(),
            layer,
            Some(namespace),
            wl_output.as_ref(),
        );
        layer_surface.set_anchor(anchor);
        layer_surface.set_keyboard_interactivity(keyboard_interactivity);
        layer_surface.set_margin(
            margin.top,
            margin.right,
            margin.bottom,
            margin.left,
        );
        layer_surface
            .set_size(size.0.unwrap_or_default(), size.1.unwrap_or_default());
        layer_surface.set_exclusive_zone(exclusive_zone);
        if !pointer_interactivity {
            let region = self
                .compositor_state
                .wl_compositor()
                .create_region(&self.queue_handle, ());
            layer_surface.set_input_region(Some(&region));
            region.destroy();
        }
        layer_surface.commit();

        let wp_viewport = self.viewporter_state.as_ref().map(|state| {
            state.get_viewport(layer_surface.wl_surface(), &self.queue_handle)
        });
        let wp_fractional_scale =
            self.fractional_scaling_manager.as_ref().map(|fsm| {
                fsm.fractional_scaling(
                    layer_surface.wl_surface(),
                    &self.queue_handle,
                )
            });

        self.layer_surfaces.push(SctkLayerSurface {
            id,
            surface: layer_surface,
            requested_size: size,
            current_size: None,
            layer,
            // builder needs to be refactored such that these fields are accessible
            anchor,
            keyboard_interactivity,
            margin,
            exclusive_zone,
            last_configure: None,
            _pending_requests: Vec::new(),
            wp_viewport,
            wp_fractional_scale,
            scale_factor: None,
        });
        Ok((id, wl_surface))
    }
    pub fn get_lock_surface(
        &mut self,
        id: window::Id,
        output: &WlOutput,
    ) -> Option<WlSurface> {
        if let Some(lock) = self.session_lock.as_ref() {
            let wl_surface =
                self.compositor_state.create_surface(&self.queue_handle);
            let session_lock_surface = lock.create_lock_surface(
                wl_surface.clone(),
                output,
                &self.queue_handle,
            );
            self.lock_surfaces.push(SctkLockSurface {
                id,
                session_lock_surface,
                last_configure: None,
            });
            Some(wl_surface)
        } else {
            None
        }
    }
}

delegate_noop!(@<T: 'static + Debug> SctkState<T>: ignore WlSubsurface);
delegate_noop!(@<T: 'static + Debug> SctkState<T>: ignore WlRegion);
