//! Write some text for your users to read.
use crate::alignment;
use crate::layout;
use crate::mouse;
use crate::renderer;
use crate::text::paragraph::{self, Paragraph};
use crate::text::{self, Fragment};
use crate::widget::tree::{self, Tree};
use crate::{
    Color, Element, Layout, Length, Pixels, Point, Rectangle, Size, Theme,
    Widget,
};

use std::borrow::Cow;

pub use text::{LineHeight, Shaping, Wrap};

/// A paragraph of text.
#[allow(missing_debug_implementations)]
pub struct Text<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fragment: Fragment<'a>,
    id: crate::widget::Id,
    size: Option<Pixels>,
    line_height: LineHeight,
    width: Length,
    height: Length,
    horizontal_alignment: alignment::Horizontal,
    vertical_alignment: alignment::Vertical,
    font: Option<Renderer::Font>,
    shaping: Shaping,
    class: Theme::Class<'a>,
    wrap: Wrap,
}

impl<'a, Theme, Renderer> Text<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    /// Create a new fragment of [`Text`] with the given contents.
    pub fn new(fragment: impl text::IntoFragment<'a>) -> Self {
        Text {
            fragment: fragment.into_fragment(),
            id: crate::widget::Id::unique(),
            size: None,
            line_height: LineHeight::default(),
            font: None,
            width: Length::Shrink,
            height: Length::Shrink,
            horizontal_alignment: alignment::Horizontal::Left,
            vertical_alignment: alignment::Vertical::Top,
            shaping: Shaping::Basic,
            class: Theme::default(),
            wrap: Default::default(),
        }
    }

    /// Sets the size of the [`Text`].
    pub fn size(mut self, size: impl Into<Pixels>) -> Self {
        self.size = Some(size.into());
        self
    }

    /// Sets the [`LineHeight`] of the [`Text`].
    pub fn line_height(mut self, line_height: impl Into<LineHeight>) -> Self {
        self.line_height = line_height.into();
        self
    }

    /// Sets the [`Font`] of the [`Text`].
    ///
    /// [`Font`]: crate::text::Renderer::Font
    pub fn font(mut self, font: impl Into<Renderer::Font>) -> Self {
        self.font = Some(font.into());
        self
    }

    /// Sets the width of the [`Text`] boundaries.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Text`] boundaries.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Centers the [`Text`], both horizontally and vertically.
    pub fn center(self) -> Self {
        self.align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
    }

    /// Sets the [`alignment::Horizontal`] of the [`Text`].
    pub fn align_x(
        mut self,
        alignment: impl Into<alignment::Horizontal>,
    ) -> Self {
        self.horizontal_alignment = alignment.into();
        self
    }

    /// Sets the [`alignment::Vertical`] of the [`Text`].
    pub fn align_y(
        mut self,
        alignment: impl Into<alignment::Vertical>,
    ) -> Self {
        self.vertical_alignment = alignment.into();
        self
    }

    /// Sets the [`Shaping`] strategy of the [`Text`].
    pub fn shaping(mut self, shaping: Shaping) -> Self {
        self.shaping = shaping;
        self
    }

    /// Sets the style of the [`Text`].
    #[must_use]
    pub fn style(mut self, style: impl Fn(&Theme) -> Style + 'a) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.class = (Box::new(style) as StyleFn<'a, Theme>).into();
        self
    }

    /// Sets the [`Color`] of the [`Text`].
    pub fn color(self, color: impl Into<Color>) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        self.color_maybe(Some(color))
    }

    /// Sets the [`Color`] of the [`Text`], if `Some`.
    pub fn color_maybe(self, color: Option<impl Into<Color>>) -> Self
    where
        Theme::Class<'a>: From<StyleFn<'a, Theme>>,
    {
        let color = color.map(Into::into);

        self.style(move |_theme| Style { color })
    }

    /// Sets the style class of the [`Text`].
    #[cfg(feature = "advanced")]
    #[must_use]
    pub fn class(mut self, class: impl Into<Theme::Class<'a>>) -> Self {
        self.class = class.into();
        self
    }

    /// Sets the [`Wrap`] mode of the [`Text`].
    pub fn wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = wrap;
        self
    }
}

/// The internal state of a [`Text`] widget.
#[derive(Debug, Default)]
pub struct State<P: Paragraph>(pub paragraph::Plain<P>);

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Text<'a, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State<Renderer::Paragraph>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::<Renderer::Paragraph>(
            paragraph::Plain::default(),
        ))
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout(
            tree.state.downcast_mut::<State<Renderer::Paragraph>>(),
            renderer,
            limits,
            self.width,
            self.height,
            &self.fragment,
            self.line_height,
            self.size,
            self.font,
            self.horizontal_alignment,
            self.vertical_alignment,
            self.shaping,
            self.wrap,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        _cursor_position: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State<Renderer::Paragraph>>();
        let style = theme.style(&self.class);

        draw(renderer, defaults, layout, state.0.raw(), style, viewport);
    }

    #[cfg(feature = "a11y")]
    fn a11y_nodes(
        &self,
        layout: Layout<'_>,
        _state: &Tree,
        _: mouse::Cursor,
    ) -> iced_accessibility::A11yTree {
        use iced_accessibility::{
            accesskit::{Live, NodeBuilder, Rect, Role},
            A11yTree,
        };

        let Rectangle {
            x,
            y,
            width,
            height,
        } = layout.bounds();
        let bounds = Rect::new(
            x as f64,
            y as f64,
            (x + width) as f64,
            (y + height) as f64,
        );

        let mut node = NodeBuilder::new(Role::Paragraph);

        // TODO is the name likely different from the content?
        node.set_name(self.fragment.to_string().into_boxed_str());
        node.set_bounds(bounds);

        // TODO make this configurable
        node.set_live(Live::Polite);
        A11yTree::leaf(node, self.id.clone())
    }

    fn id(&self) -> Option<crate::widget::Id> {
        Some(self.id.clone())
    }

    fn set_id(&mut self, id: crate::widget::Id) {
        self.id = id
    }
}

/// Produces the [`layout::Node`] of a [`Text`] widget.
pub fn layout<Renderer>(
    state: &mut State<Renderer::Paragraph>,
    renderer: &Renderer,
    limits: &layout::Limits,
    width: Length,
    height: Length,
    content: &str,
    line_height: LineHeight,
    size: Option<Pixels>,
    font: Option<Renderer::Font>,
    horizontal_alignment: alignment::Horizontal,
    vertical_alignment: alignment::Vertical,
    shaping: Shaping,
    wrap: Wrap,
) -> layout::Node
where
    Renderer: text::Renderer,
{
    layout::sized(limits, width, height, |limits| {
        let bounds = limits.max();

        let size = size.unwrap_or_else(|| renderer.default_size());
        let font = font.unwrap_or_else(|| renderer.default_font());

        let State(ref mut paragraph) = state;

        paragraph.update(text::Text {
            content,
            bounds,
            size,
            line_height,
            font,
            horizontal_alignment,
            vertical_alignment,
            shaping,
            wrap,
        });

        paragraph.min_bounds()
    })
}

/// Draws text using the same logic as the [`Text`] widget.
///
/// Specifically:
///
/// * If no `size` is provided, the default text size of the `Renderer` will be
///   used.
/// * If no `color` is provided, the [`renderer::Style::text_color`] will be
///   used.
/// * The alignment attributes do not affect the position of the bounds of the
///   [`Layout`].
pub fn draw<Renderer>(
    renderer: &mut Renderer,
    style: &renderer::Style,
    layout: Layout<'_>,
    paragraph: &Renderer::Paragraph,
    appearance: Style,
    viewport: &Rectangle,
) where
    Renderer: text::Renderer,
{
    let bounds = layout.bounds();

    let x = match paragraph.horizontal_alignment() {
        alignment::Horizontal::Left => bounds.x,
        alignment::Horizontal::Center => bounds.center_x(),
        alignment::Horizontal::Right => bounds.x + bounds.width,
    };

    let y = match paragraph.vertical_alignment() {
        alignment::Vertical::Top => bounds.y,
        alignment::Vertical::Center => bounds.center_y(),
        alignment::Vertical::Bottom => bounds.y + bounds.height,
    };

    renderer.fill_paragraph(
        paragraph,
        Point::new(x, y),
        appearance.color.unwrap_or(style.text_color),
        *viewport,
    );
}

impl<'a, Message, Theme, Renderer> From<Text<'a, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(
        text: Text<'a, Theme, Renderer>,
    ) -> Element<'a, Message, Theme, Renderer> {
        Element::new(text)
    }
}

// impl<'a, Theme, Renderer> Clone for Text<'a, Theme, Renderer>
// where
//     Renderer: text::Renderer,
// {
//     fn clone(&self) -> Self {
//         Self {
//             id: self.id.clone(),
//             content: self.content.clone(),
//             size: self.size,
//             line_height: self.line_height,
//             width: self.width,
//             height: self.height,
//             horizontal_alignment: self.horizontal_alignment,
//             vertical_alignment: self.vertical_alignment,
//             font: self.font,
//             style: self.style,
//             shaping: self.shaping,
//             wrap: self.wrap,
//         }
//     }
// }
// TODO(POP): Clone no longer can be implemented because of style being a Box(style)

impl<'a, Theme, Renderer> From<&'a str> for Text<'a, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer,
{
    fn from(content: &'a str) -> Self {
        Self::new(content)
    }
}

impl<'a, Message, Theme, Renderer> From<&'a str>
    for Element<'a, Message, Theme, Renderer>
where
    Theme: Catalog + 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(content: &'a str) -> Self {
        Text::from(content).into()
    }
}

/// The appearance of some text.
#[derive(Debug, Clone, Copy, Default)]
pub struct Style {
    /// The [`Color`] of the text.
    ///
    /// The default, `None`, means using the inherited color.
    pub color: Option<Color>,
}

/// The theme catalog of a [`Text`].
pub trait Catalog: Sized {
    /// The item class of this [`Catalog`].
    type Class<'a>;

    /// The default class produced by this [`Catalog`].
    fn default<'a>() -> Self::Class<'a>;

    /// The [`Style`] of a class with the given status.
    fn style(&self, item: &Self::Class<'_>) -> Style;
}

/// A styling function for a [`Text`].
///
/// This is just a boxed closure: `Fn(&Theme, Status) -> Style`.
pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|_theme| Style::default())
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

/// The default text styling; color is inherited.
pub fn default(_theme: &Theme) -> Style {
    Style { color: None }
}

/// Text with the default base color.
pub fn base(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().text),
    }
}

/// Text conveying some important information, like an action.
pub fn primary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().primary),
    }
}

/// Text conveying some secondary information, like a footnote.
pub fn secondary(theme: &Theme) -> Style {
    Style {
        color: Some(theme.extended_palette().secondary.strong.color),
    }
}

/// Text conveying some positive information, like a successful event.
pub fn success(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().success),
    }
}

/// Text conveying some negative information, like an error.
pub fn danger(theme: &Theme) -> Style {
    Style {
        color: Some(theme.palette().danger),
    }
}
