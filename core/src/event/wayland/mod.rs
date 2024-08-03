mod data_device;
mod layer;
mod output;
mod popup;
mod seat;
mod session_lock;
mod window;

use crate::{time::Instant, window::Id};
use sctk::reexports::client::protocol::{
    wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
};

pub use data_device::*;
pub use layer::*;
pub use output::*;
pub use popup::*;
pub use seat::*;
pub use session_lock::*;
pub use window::*;

/// wayland events
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// layer surface event
    Layer(LayerEvent, WlSurface, Id),
    /// popup event
    Popup(PopupEvent, WlSurface, Id),
    /// output event
    Output(OutputEvent, WlOutput),
    /// window event
    Window(WindowEvent, WlSurface, Id),
    /// Seat Event
    Seat(SeatEvent, WlSeat),
    /// Data Device event
    DataSource(DataSourceEvent),
    /// Dnd Offer events
    DndOffer(DndOfferEvent),
    /// Selection Offer events
    SelectionOffer(SelectionOfferEvent),
    /// Session lock events
    SessionLock(SessionLockEvent),
    /// Frame events
    Frame(Instant, WlSurface, Id),
}

impl Event {
    /// Translate the event by some vector
    pub fn translate(&mut self, vector: crate::vector::Vector) {
        match self {
            Event::DndOffer(DndOfferEvent::Enter { x, y, .. }) => {
                *x += vector.x as f64;
                *y += vector.y as f64;
            }
            Event::DndOffer(DndOfferEvent::Motion { x, y }) => {
                *x += vector.x as f64;
                *y += vector.y as f64;
            }
            _ => {}
        }
    }
}
