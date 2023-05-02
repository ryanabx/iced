//! Write your own renderer.
#[cfg(debug_assertions)]
mod null;

#[cfg(debug_assertions)]
pub use null::Null;

use crate::{Background, Border, Color, Rectangle, Shadow, Size, Vector};

/// A component that can be used by widgets to draw themselves on a screen.
pub trait Renderer: Sized {
    /// Draws the primitives recorded in the given closure in a new layer.
    ///
    /// The layer will clip its contents to the provided `bounds`.
    fn with_layer(&mut self, bounds: Rectangle, f: impl FnOnce(&mut Self));

    /// Applies a `translation` to the primitives recorded in the given closure.
    fn with_translation(
        &mut self,
        translation: Vector,
        f: impl FnOnce(&mut Self),
    );

    /// Fills a [`Quad`] with the provided [`Background`].
    fn fill_quad(&mut self, quad: Quad, background: impl Into<Background>);

    /// Clears all of the recorded primitives in the [`Renderer`].
    fn clear(&mut self);
}

/// A polygon with four sides.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// The bounds of the [`Quad`].
    pub bounds: Rectangle,

    /// The [`Border`] of the [`Quad`].
    pub border: Border,

    /// The [`Shadow`] of the [`Quad`].
    pub shadow: Shadow,
}

impl Default for Quad {
    fn default() -> Self {
        Self {
            bounds: Rectangle::with_size(Size::ZERO),
            border: Border::default(),
            shadow: Shadow::default(),
        }
    }
}

/// The styling attributes of a [`Renderer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    /// The color to apply to symbolic icons.
    pub icon_color: Color,
    /// The text color
    pub text_color: Color,
    /// The scale factor
    pub scale_factor: f64,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            icon_color: Color::BLACK,
            text_color: Color::BLACK,
            scale_factor: 1.0,
        }
    }
}
