//! A container for capturing mouse events.

use crate::core::event::wayland::DndOfferEvent;
use crate::core::event::{self, Event, PlatformSpecific};
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::OperationOutputWrapper;
use crate::core::widget::{tree, Operation, Tree};
use crate::core::{
    overlay, Clipboard, Element, Layout, Length, Point, Rectangle, Shell,
    Vector, Widget,
};
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;

use std::u32;

/// Emit messages on mouse events.
#[allow(missing_debug_implementations)]
pub struct DndListener<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,

    /// Sets the message to emit on a drag enter.
    on_enter:
        Option<Box<dyn Fn(DndAction, Vec<String>, (f32, f32)) -> Message + 'a>>,

    /// Sets the message to emit on a drag motion.
    /// x and y are the coordinates of the pointer relative to the widget in the range (0.0, 1.0)
    on_motion: Option<Box<dyn Fn(f32, f32) -> Message + 'a>>,

    /// Sets the message to emit on a drag exit.
    on_exit: Option<Message>,

    /// Sets the message to emit on a drag drop.
    on_drop: Option<Message>,

    /// Sets the message to emit on a drag mime type event.
    on_mime_type: Option<Box<dyn Fn(String) -> Message + 'a>>,

    /// Sets the message to emit on a drag action event.
    on_source_actions: Option<Box<dyn Fn(DndAction) -> Message + 'a>>,

    /// Sets the message to emit on a drag action event.
    on_selected_action: Option<Box<dyn Fn(DndAction) -> Message + 'a>>,

    /// Sets the message to emit on a Data event.
    on_data: Option<Box<dyn Fn(String, Vec<u8>) -> Message + 'a>>,
}

impl<'a, Message, Theme, Renderer> DndListener<'a, Message, Theme, Renderer> {
    /// The message to emit on a drag enter.
    #[must_use]
    pub fn on_enter(
        mut self,
        message: impl Fn(DndAction, Vec<String>, (f32, f32)) -> Message + 'a,
    ) -> Self {
        self.on_enter = Some(Box::new(message));
        self
    }

    /// The message to emit on a drag motion.
    #[must_use]
    pub fn on_motion(
        mut self,
        message: impl Fn(f32, f32) -> Message + 'a,
    ) -> Self {
        self.on_motion = Some(Box::new(message));
        self
    }

    /// The message to emit on a selected drag action.
    #[must_use]
    pub fn on_selected_action(
        mut self,
        message: impl Fn(DndAction) -> Message + 'a,
    ) -> Self {
        self.on_selected_action = Some(Box::new(message));
        self
    }

    /// The message to emit on a drag exit.
    #[must_use]
    pub fn on_exit(mut self, message: Message) -> Self {
        self.on_exit = Some(message);
        self
    }

    /// The message to emit on a drag drop.
    #[must_use]
    pub fn on_drop(mut self, message: Message) -> Self {
        self.on_drop = Some(message);
        self
    }

    /// The message to emit on a drag mime type event.
    #[must_use]
    pub fn on_mime_type(
        mut self,
        message: impl Fn(String) -> Message + 'a,
    ) -> Self {
        self.on_mime_type = Some(Box::new(message));
        self
    }

    /// The message to emit on a drag action event.
    #[must_use]
    pub fn on_action(
        mut self,
        message: impl Fn(DndAction) -> Message + 'a,
    ) -> Self {
        self.on_source_actions = Some(Box::new(message));
        self
    }

    /// The message to emit on a drag read data event.
    #[must_use]
    pub fn on_data(
        mut self,
        message: impl Fn(String, Vec<u8>) -> Message + 'a,
    ) -> Self {
        self.on_data = Some(Box::new(message));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum DndState {
    #[default]
    None,
    External(DndAction, Vec<String>),
    Hovered(DndAction, Vec<String>),
    Dropped,
}

/// Local state of the [`DndListener`].
#[derive(Default)]
struct State {
    dnd: DndState,
}

impl<'a, Message, Theme, Renderer> DndListener<'a, Message, Theme, Renderer> {
    /// Creates an empty [`DndListener`].
    pub fn new(
        content: impl Into<Element<'a, Message, Theme, Renderer>>,
    ) -> Self {
        DndListener {
            content: content.into(),
            on_enter: None,
            on_motion: None,
            on_exit: None,
            on_drop: None,
            on_mime_type: None,
            on_source_actions: None,
            on_selected_action: None,
            on_data: None,
        }
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for DndListener<'a, Message, Theme, Renderer>
where
    Renderer: crate::core::Renderer,
    Message: Clone,
{
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = self.size();

        layout(
            renderer,
            limits,
            size.width,
            size.height,
            u32::MAX,
            u32::MAX,
            |renderer, limits| {
                self.content.as_widget().layout(
                    &mut tree.children[0],
                    renderer,
                    limits,
                )
            },
        )
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation<OperationOutputWrapper<Message>>,
    ) {
        operation.container(None, layout.bounds(), &mut |operation| {
            self.content.as_widget().operate(
                &mut tree.children[0],
                layout.children().next().unwrap(),
                renderer,
                operation,
            );
        });
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        if let event::Status::Captured = self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event.clone(),
            layout.children().next().unwrap(),
            cursor_position,
            renderer,
            clipboard,
            shell,
            viewport,
        ) {
            return event::Status::Captured;
        }

        update(
            self,
            &event,
            layout,
            shell,
            tree.state.downcast_mut::<State>(),
        )
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor_position,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        renderer_style: &renderer::Style,
        layout: Layout<'_>,
        cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            renderer_style,
            layout.children().next().unwrap(),
            cursor_position,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            translation, // TODO(POP): Vector arg added, is this correct?
        )
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> iced_renderer::core::Size<Length> {
        self.content.as_widget().size()
    }
}

impl<'a, Message, Theme, Renderer>
    From<DndListener<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Renderer: 'a + crate::core::Renderer,
    Theme: 'a,
{
    fn from(
        listener: DndListener<'a, Message, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(listener)
    }
}

/// Processes the given [`Event`] and updates the [`State`] of an [`DndListener`]
/// accordingly.
fn update<Message: Clone, Renderer, Theme>(
    widget: &mut DndListener<'_, Message, Theme, Renderer>,
    event: &Event,
    layout: Layout<'_>,
    shell: &mut Shell<'_, Message>,
    state: &mut State,
) -> event::Status {
    match event {
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::Enter {
                x,
                y,
                mime_types,
            }),
        )) => {
            let bounds = layout.bounds();
            let p = Point {
                x: *x as f32,
                y: *y as f32,
            };
            if layout.bounds().contains(p) {
                state.dnd =
                    DndState::Hovered(DndAction::empty(), mime_types.clone());
                if let Some(message) = widget.on_enter.as_ref() {
                    let normalized_x: f32 = (p.x - bounds.x) / bounds.width;
                    let normalized_y: f32 = (p.y - bounds.y) / bounds.height;
                    shell.publish(message(
                        DndAction::empty(),
                        mime_types.clone(),
                        (normalized_x, normalized_y),
                    ));
                    return event::Status::Captured;
                }
            } else {
                state.dnd =
                    DndState::External(DndAction::empty(), mime_types.clone());
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::Motion { x, y }),
        )) => {
            let bounds = layout.bounds();
            let p = Point {
                x: *x as f32,
                y: *y as f32,
            };
            // motion can trigger an enter, motion or leave event on the widget
            if let DndState::Hovered(action, mime_types) = &state.dnd {
                if !bounds.contains(p) {
                    state.dnd = DndState::External(*action, mime_types.clone());
                    if let Some(message) = widget.on_exit.clone() {
                        shell.publish(message);
                        return event::Status::Captured;
                    }
                } else if let Some(message) = widget.on_motion.as_ref() {
                    let normalized_x: f32 = (p.x - bounds.x) / bounds.width;
                    let normalized_y: f32 = (p.y - bounds.y) / bounds.height;
                    shell.publish(message(normalized_x, normalized_y));
                    return event::Status::Captured;
                }
            } else if bounds.contains(p) {
                state.dnd = match &state.dnd {
                    DndState::External(a, m) => {
                        DndState::Hovered(*a, m.clone())
                    }
                    _ => DndState::Hovered(DndAction::empty(), vec![]),
                };
                let (action, mime_types) = match &state.dnd {
                    DndState::Hovered(action, mime_types) => {
                        (action, mime_types)
                    }
                    _ => return event::Status::Ignored,
                };

                if let Some(message) = widget.on_enter.as_ref() {
                    let normalized_x: f32 = (p.x - bounds.x) / bounds.width;
                    let normalized_y: f32 = (p.y - bounds.y) / bounds.height;
                    shell.publish(message(
                        *action,
                        mime_types.clone(),
                        (normalized_x, normalized_y),
                    ));
                    return event::Status::Captured;
                }
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::Leave),
        )) => {
            if matches!(state.dnd, DndState::None | DndState::External(..)) {
                return event::Status::Ignored;
            }

            if !matches!(state.dnd, DndState::Dropped) {
                state.dnd = DndState::None;
            }

            if let Some(message) = widget.on_exit.clone() {
                shell.publish(message);
                return event::Status::Captured;
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::DropPerformed),
        )) => {
            if matches!(state.dnd, DndState::Hovered(..)) {
                state.dnd = DndState::Dropped;
                if let Some(message) = widget.on_drop.clone() {
                    shell.publish(message);
                    return event::Status::Captured;
                }
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::DndData {
                mime_type,
                data,
            }),
        )) => {
            match &mut state.dnd {
                DndState::Hovered(_, mime_types) => {
                    if !mime_types.contains(mime_type) {
                        return event::Status::Ignored;
                    }
                }
                DndState::None | DndState::External(..) => {
                    return event::Status::Ignored
                }
                DndState::Dropped => {}
            };
            if let Some(message) = widget.on_data.as_ref() {
                shell.publish(message(mime_type.clone(), data.clone()));
                return event::Status::Captured;
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::SourceActions(
                actions,
            )),
        )) => {
            match &mut state.dnd {
                DndState::Hovered(ref mut action, _) => *action = *actions,
                DndState::External(ref mut action, _) => *action = *actions,
                DndState::Dropped => {}
                DndState::None => {
                    state.dnd = DndState::External(*actions, vec![])
                }
            };
            if let Some(message) = widget.on_source_actions.as_ref() {
                shell.publish(message(*actions));
                return event::Status::Captured;
            }
        }
        Event::PlatformSpecific(PlatformSpecific::Wayland(
            event::wayland::Event::DndOffer(DndOfferEvent::SelectedAction(
                action,
            )),
        )) => {
            if matches!(state.dnd, DndState::None | DndState::External(..)) {
                return event::Status::Ignored;
            }

            if let Some(message) = widget.on_selected_action.as_ref() {
                shell.publish(message(*action));
                return event::Status::Captured;
            }
        }
        _ => {}
    };
    event::Status::Ignored
}

/// Computes the layout of a [`DndListener`].
pub fn layout<Renderer>(
    renderer: &Renderer,
    limits: &layout::Limits,
    width: Length,
    height: Length,
    max_height: u32,
    max_width: u32,
    layout_content: impl FnOnce(&Renderer, &layout::Limits) -> layout::Node,
) -> layout::Node {
    let limits = limits
        .loose()
        .max_height(max_height as f32)
        .max_width(max_width as f32)
        .width(width)
        .height(height);

    let content = layout_content(renderer, &limits);
    let size = limits.resolve(width, height, content.size());

    layout::Node::with_children(size, vec![content])
}
