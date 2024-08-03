//! Create and run iced applications step by step.
//!
//! # Example
//! ```no_run
//! use iced::widget::{button, column, text, Column};
//! use iced::Theme;
//!
//! pub fn main() -> iced::Result {
//!     iced::program("A counter", update, view)
//!         .theme(|_| Theme::Dark)
//!         .centered()
//!         .run()
//! }
//!
//! #[derive(Debug, Clone)]
//! enum Message {
//!     Increment,
//! }
//!
//! fn update(value: &mut u64, message: Message) {
//!     match message {
//!         Message::Increment => *value += 1,
//!     }
//! }
//!
//! fn view(value: &u64) -> Column<Message> {
//!     column![
//!         text(value),
//!         button("+").on_press(Message::Increment),
//!     ]
//! }
//! ```
use crate::application::Application;
use crate::core::text;
use crate::executor::{self, Executor};
use crate::graphics::compositor;
use crate::window;
use crate::{Command, Element, Font, Result, Settings, Size, Subscription};

pub use crate::application::{Appearance, DefaultStyle};

use std::borrow::Cow;

/// Creates an iced [`Program`] given its title, update, and view logic.
///
/// # Example
/// ```no_run
/// use iced::widget::{button, column, text, Column};
///
/// pub fn main() -> iced::Result {
///     iced::program("A counter", update, view).run()
/// }
///
/// #[derive(Debug, Clone)]
/// enum Message {
///     Increment,
/// }
///
/// fn update(value: &mut u64, message: Message) {
///     match message {
///         Message::Increment => *value += 1,
///     }
/// }
///
/// fn view(value: &u64) -> Column<Message> {
///     column![
///         text(value),
///         button("+").on_press(Message::Increment),
///     ]
/// }
/// ```
pub fn program<State, Message, Theme, Renderer>(
    title: impl Title<State>,
    update: impl Update<State, Message>,
    view: impl for<'a> self::View<'a, State, Message, Theme, Renderer>,
) -> Program<impl Definition<State = State, Message = Message, Theme = Theme>>
where
    State: 'static,
    Message: Send + std::fmt::Debug + 'static,
    Theme: Default + DefaultStyle,
    Renderer: self::Renderer,
{
    use std::marker::PhantomData;

    struct Application<State, Message, Theme, Renderer, Update, View> {
        update: Update,
        view: View,
        _state: PhantomData<State>,
        _message: PhantomData<Message>,
        _theme: PhantomData<Theme>,
        _renderer: PhantomData<Renderer>,
    }

    impl<State, Message, Theme, Renderer, Update, View> Definition
        for Application<State, Message, Theme, Renderer, Update, View>
    where
        Message: Send + std::fmt::Debug + 'static,
        Theme: Default + DefaultStyle,
        Renderer: self::Renderer,
        Update: self::Update<State, Message>,
        View: for<'a> self::View<'a, State, Message, Theme, Renderer>,
    {
        type State = State;
        type Message = Message;
        type Theme = Theme;
        type Renderer = Renderer;
        type Executor = executor::Default;

        fn load(&self) -> Command<Self::Message> {
            Command::none()
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.update.update(state, message).into()
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.view.view(state).into()
        }
    }

    Program {
        raw: Application {
            update,
            view,
            _state: PhantomData,
            _message: PhantomData,
            _theme: PhantomData,
            _renderer: PhantomData,
        },
        settings: Settings::default(),
    }
    .title(title)
}

/// The underlying definition and configuration of an iced application.
///
/// You can use this API to create and run iced applications
/// step by step—without coupling your logic to a trait
/// or a specific type.
///
/// You can create a [`Program`] with the [`program`] helper.
///
/// [`run`]: Program::run
#[derive(Debug)]
pub struct Program<P: Definition> {
    raw: P,
    settings: Settings,
}

impl<P: Definition> Program<P> {
    /// Runs the underlying [`Application`] of the [`Program`].
    ///
    /// The state of the [`Program`] must implement [`Default`].
    /// If your state does not implement [`Default`], use [`run_with`]
    /// instead.
    ///
    /// [`run_with`]: Self::run_with
    pub fn run(self) -> Result
    where
        Self: 'static,
        P::State: Default,
    {
        self.run_with(P::State::default)
    }

    /// Runs the underlying [`Application`] of the [`Program`] with a
    /// closure that creates the initial state.
    pub fn run_with(
        self,
        initialize: impl Fn() -> P::State + Clone + 'static,
    ) -> Result
    where
        Self: 'static,
    {
        use std::marker::PhantomData;

        struct Instance<P: Definition, I> {
            program: P,
            state: P::State,
            _initialize: PhantomData<I>,
        }

        impl<P: Definition, I: Fn() -> P::State> Application for Instance<P, I> {
            type Message = P::Message;
            type Theme = P::Theme;
            type Renderer = P::Renderer;
            type Flags = (P, I);
            type Executor = P::Executor;

            fn new(
                (program, initialize): Self::Flags,
            ) -> (Self, Command<Self::Message>) {
                let state = initialize();
                let command = program.load();

                (
                    Self {
                        program,
                        state,
                        _initialize: PhantomData,
                    },
                    command,
                )
            }

            fn title(&self) -> String {
                self.program.title(&self.state)
            }

            fn update(
                &mut self,
                message: Self::Message,
            ) -> Command<Self::Message> {
                self.program.update(&mut self.state, message)
            }

            fn view(
                &self,
            ) -> crate::Element<'_, Self::Message, Self::Theme, Self::Renderer>
            {
                self.program.view(&self.state)
            }

            fn subscription(&self) -> Subscription<Self::Message> {
                self.program.subscription(&self.state)
            }

            fn theme(&self) -> Self::Theme {
                self.program.theme(&self.state)
            }

            fn style(&self, theme: &Self::Theme) -> Appearance {
                self.program.style(&self.state, theme)
            }
        }

        let Self { raw, settings } = self;

        Instance::run(Settings {
            flags: (raw, initialize),
            id: settings.id,
            window: settings.window,
            fonts: settings.fonts,
            default_font: settings.default_font,
            default_text_size: settings.default_text_size,
            antialiasing: settings.antialiasing,
            exit_on_close_request: settings.exit_on_close_request,
        })
    }

    /// Sets the [`Settings`] that will be used to run the [`Program`].
    pub fn settings(self, settings: Settings) -> Self {
        Self { settings, ..self }
    }

    /// Sets the [`Settings::antialiasing`] of the [`Program`].
    pub fn antialiasing(self, antialiasing: bool) -> Self {
        Self {
            settings: Settings {
                antialiasing,
                ..self.settings
            },
            ..self
        }
    }

    /// Sets the default [`Font`] of the [`Program`].
    pub fn default_font(self, default_font: Font) -> Self {
        Self {
            settings: Settings {
                default_font,
                ..self.settings
            },
            ..self
        }
    }

    /// Adds a font to the list of fonts that will be loaded at the start of the [`Program`].
    pub fn font(mut self, font: impl Into<Cow<'static, [u8]>>) -> Self {
        self.settings.fonts.push(font.into());
        self
    }

    /// Sets the [`window::Settings::position`] to [`window::Position::Centered`] in the [`Program`].
    pub fn centered(self) -> Self {
        Self {
            settings: Settings {
                window: window::Settings {
                    position: window::Position::Centered,
                    ..self.settings.window
                },
                ..self.settings
            },
            ..self
        }
    }

    /// Sets the [`window::Settings::exit_on_close_request`] of the [`Program`].
    pub fn exit_on_close_request(self, exit_on_close_request: bool) -> Self {
        Self {
            settings: Settings {
                window: window::Settings {
                    exit_on_close_request,
                    ..self.settings.window
                },
                ..self.settings
            },
            ..self
        }
    }

    /// Sets the [`window::Settings::size`] of the [`Program`].
    pub fn window_size(self, size: impl Into<Size>) -> Self {
        Self {
            settings: Settings {
                window: window::Settings {
                    size: size.into(),
                    ..self.settings.window
                },
                ..self.settings
            },
            ..self
        }
    }

    /// Sets the [`window::Settings::transparent`] of the [`Program`].
    pub fn transparent(self, transparent: bool) -> Self {
        Self {
            settings: Settings {
                window: window::Settings {
                    transparent,
                    ..self.settings.window
                },
                ..self.settings
            },
            ..self
        }
    }

    /// Sets the [`Title`] of the [`Program`].
    pub(crate) fn title(
        self,
        title: impl Title<P::State>,
    ) -> Program<
        impl Definition<State = P::State, Message = P::Message, Theme = P::Theme>,
    > {
        Program {
            raw: with_title(self.raw, title),
            settings: self.settings,
        }
    }

    /// Runs the [`Command`] produced by the closure at startup.
    pub fn load(
        self,
        f: impl Fn() -> Command<P::Message>,
    ) -> Program<
        impl Definition<State = P::State, Message = P::Message, Theme = P::Theme>,
    > {
        Program {
            raw: with_load(self.raw, f),
            settings: self.settings,
        }
    }

    /// Sets the subscription logic of the [`Program`].
    pub fn subscription(
        self,
        f: impl Fn(&P::State) -> Subscription<P::Message>,
    ) -> Program<
        impl Definition<State = P::State, Message = P::Message, Theme = P::Theme>,
    > {
        Program {
            raw: with_subscription(self.raw, f),
            settings: self.settings,
        }
    }

    /// Sets the theme logic of the [`Program`].
    pub fn theme(
        self,
        f: impl Fn(&P::State) -> P::Theme,
    ) -> Program<
        impl Definition<State = P::State, Message = P::Message, Theme = P::Theme>,
    > {
        Program {
            raw: with_theme(self.raw, f),
            settings: self.settings,
        }
    }

    /// Sets the style logic of the [`Program`].
    pub fn style(
        self,
        f: impl Fn(&P::State, &P::Theme) -> Appearance,
    ) -> Program<
        impl Definition<State = P::State, Message = P::Message, Theme = P::Theme>,
    > {
        Program {
            raw: with_style(self.raw, f),
            settings: self.settings,
        }
    }
}

/// The internal definition of a [`Program`].
///
/// You should not need to implement this trait directly. Instead, use the
/// methods available in the [`Program`] struct.
#[allow(missing_docs)]
pub trait Definition: Sized {
    /// The state of the program.
    type State;

    /// The message of the program.
    type Message: Send + std::fmt::Debug + 'static;

    /// The theme of the program.
    type Theme: Default + DefaultStyle;

    /// The renderer of the program.
    type Renderer: Renderer;

    /// The executor of the program.
    type Executor: Executor;

    fn load(&self) -> Command<Self::Message>;

    fn update(
        &self,
        state: &mut Self::State,
        message: Self::Message,
    ) -> Command<Self::Message>;

    fn view<'a>(
        &self,
        state: &'a Self::State,
    ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer>;

    fn title(&self, _state: &Self::State) -> String {
        String::from("A cool iced application!")
    }

    fn subscription(
        &self,
        _state: &Self::State,
    ) -> Subscription<Self::Message> {
        Subscription::none()
    }

    fn theme(&self, _state: &Self::State) -> Self::Theme {
        Self::Theme::default()
    }

    fn style(&self, _state: &Self::State, theme: &Self::Theme) -> Appearance {
        DefaultStyle::default_style(theme)
    }
}

fn with_title<P: Definition>(
    program: P,
    title: impl Title<P::State>,
) -> impl Definition<State = P::State, Message = P::Message, Theme = P::Theme> {
    struct WithTitle<P, Title> {
        program: P,
        title: Title,
    }

    impl<P, Title> Definition for WithTitle<P, Title>
    where
        P: Definition,
        Title: self::Title<P::State>,
    {
        type State = P::State;
        type Message = P::Message;
        type Theme = P::Theme;
        type Renderer = P::Renderer;
        type Executor = P::Executor;

        fn load(&self) -> Command<Self::Message> {
            self.program.load()
        }

        fn title(&self, state: &Self::State) -> String {
            self.title.title(state)
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.program.update(state, message)
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.program.view(state)
        }

        fn theme(&self, state: &Self::State) -> Self::Theme {
            self.program.theme(state)
        }

        fn subscription(
            &self,
            state: &Self::State,
        ) -> Subscription<Self::Message> {
            self.program.subscription(state)
        }

        fn style(
            &self,
            state: &Self::State,
            theme: &Self::Theme,
        ) -> Appearance {
            self.program.style(state, theme)
        }
    }

    WithTitle { program, title }
}

fn with_load<P: Definition>(
    program: P,
    f: impl Fn() -> Command<P::Message>,
) -> impl Definition<State = P::State, Message = P::Message, Theme = P::Theme> {
    struct WithLoad<P, F> {
        program: P,
        load: F,
    }

    impl<P: Definition, F> Definition for WithLoad<P, F>
    where
        F: Fn() -> Command<P::Message>,
    {
        type State = P::State;
        type Message = P::Message;
        type Theme = P::Theme;
        type Renderer = P::Renderer;
        type Executor = executor::Default;

        fn load(&self) -> Command<Self::Message> {
            Command::batch([self.program.load(), (self.load)()])
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.program.update(state, message)
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.program.view(state)
        }

        fn title(&self, state: &Self::State) -> String {
            self.program.title(state)
        }

        fn subscription(
            &self,
            state: &Self::State,
        ) -> Subscription<Self::Message> {
            self.program.subscription(state)
        }

        fn theme(&self, state: &Self::State) -> Self::Theme {
            self.program.theme(state)
        }

        fn style(
            &self,
            state: &Self::State,
            theme: &Self::Theme,
        ) -> Appearance {
            self.program.style(state, theme)
        }
    }

    WithLoad { program, load: f }
}

fn with_subscription<P: Definition>(
    program: P,
    f: impl Fn(&P::State) -> Subscription<P::Message>,
) -> impl Definition<State = P::State, Message = P::Message, Theme = P::Theme> {
    struct WithSubscription<P, F> {
        program: P,
        subscription: F,
    }

    impl<P: Definition, F> Definition for WithSubscription<P, F>
    where
        F: Fn(&P::State) -> Subscription<P::Message>,
    {
        type State = P::State;
        type Message = P::Message;
        type Theme = P::Theme;
        type Renderer = P::Renderer;
        type Executor = executor::Default;

        fn subscription(
            &self,
            state: &Self::State,
        ) -> Subscription<Self::Message> {
            (self.subscription)(state)
        }

        fn load(&self) -> Command<Self::Message> {
            self.program.load()
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.program.update(state, message)
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.program.view(state)
        }

        fn title(&self, state: &Self::State) -> String {
            self.program.title(state)
        }

        fn theme(&self, state: &Self::State) -> Self::Theme {
            self.program.theme(state)
        }

        fn style(
            &self,
            state: &Self::State,
            theme: &Self::Theme,
        ) -> Appearance {
            self.program.style(state, theme)
        }
    }

    WithSubscription {
        program,
        subscription: f,
    }
}

fn with_theme<P: Definition>(
    program: P,
    f: impl Fn(&P::State) -> P::Theme,
) -> impl Definition<State = P::State, Message = P::Message, Theme = P::Theme> {
    struct WithTheme<P, F> {
        program: P,
        theme: F,
    }

    impl<P: Definition, F> Definition for WithTheme<P, F>
    where
        F: Fn(&P::State) -> P::Theme,
    {
        type State = P::State;
        type Message = P::Message;
        type Theme = P::Theme;
        type Renderer = P::Renderer;
        type Executor = P::Executor;

        fn theme(&self, state: &Self::State) -> Self::Theme {
            (self.theme)(state)
        }

        fn load(&self) -> Command<Self::Message> {
            self.program.load()
        }

        fn title(&self, state: &Self::State) -> String {
            self.program.title(state)
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.program.update(state, message)
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.program.view(state)
        }

        fn subscription(
            &self,
            state: &Self::State,
        ) -> Subscription<Self::Message> {
            self.program.subscription(state)
        }

        fn style(
            &self,
            state: &Self::State,
            theme: &Self::Theme,
        ) -> Appearance {
            self.program.style(state, theme)
        }
    }

    WithTheme { program, theme: f }
}

fn with_style<P: Definition>(
    program: P,
    f: impl Fn(&P::State, &P::Theme) -> Appearance,
) -> impl Definition<State = P::State, Message = P::Message, Theme = P::Theme> {
    struct WithStyle<P, F> {
        program: P,
        style: F,
    }

    impl<P: Definition, F> Definition for WithStyle<P, F>
    where
        F: Fn(&P::State, &P::Theme) -> Appearance,
    {
        type State = P::State;
        type Message = P::Message;
        type Theme = P::Theme;
        type Renderer = P::Renderer;
        type Executor = P::Executor;

        fn style(
            &self,
            state: &Self::State,
            theme: &Self::Theme,
        ) -> Appearance {
            (self.style)(state, theme)
        }

        fn load(&self) -> Command<Self::Message> {
            self.program.load()
        }

        fn title(&self, state: &Self::State) -> String {
            self.program.title(state)
        }

        fn update(
            &self,
            state: &mut Self::State,
            message: Self::Message,
        ) -> Command<Self::Message> {
            self.program.update(state, message)
        }

        fn view<'a>(
            &self,
            state: &'a Self::State,
        ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
            self.program.view(state)
        }

        fn subscription(
            &self,
            state: &Self::State,
        ) -> Subscription<Self::Message> {
            self.program.subscription(state)
        }

        fn theme(&self, state: &Self::State) -> Self::Theme {
            self.program.theme(state)
        }
    }

    WithStyle { program, style: f }
}

/// The title logic of some [`Program`].
///
/// This trait is implemented both for `&static str` and
/// any closure `Fn(&State) -> String`.
///
/// This trait allows the [`program`] builder to take any of them.
pub trait Title<State> {
    /// Produces the title of the [`Program`].
    fn title(&self, state: &State) -> String;
}

impl<State> Title<State> for &'static str {
    fn title(&self, _state: &State) -> String {
        self.to_string()
    }
}

impl<T, State> Title<State> for T
where
    T: Fn(&State) -> String,
{
    fn title(&self, state: &State) -> String {
        self(state)
    }
}

/// The update logic of some [`Program`].
///
/// This trait allows the [`program`] builder to take any closure that
/// returns any `Into<Command<Message>>`.
pub trait Update<State, Message> {
    /// Processes the message and updates the state of the [`Program`].
    fn update(
        &self,
        state: &mut State,
        message: Message,
    ) -> impl Into<Command<Message>>;
}

impl<T, State, Message, C> Update<State, Message> for T
where
    T: Fn(&mut State, Message) -> C,
    C: Into<Command<Message>>,
{
    fn update(
        &self,
        state: &mut State,
        message: Message,
    ) -> impl Into<Command<Message>> {
        self(state, message)
    }
}

/// The view logic of some [`Program`].
///
/// This trait allows the [`program`] builder to take any closure that
/// returns any `Into<Element<'_, Message>>`.
pub trait View<'a, State, Message, Theme, Renderer> {
    /// Produces the widget of the [`Program`].
    fn view(
        &self,
        state: &'a State,
    ) -> impl Into<Element<'a, Message, Theme, Renderer>>;
}

impl<'a, T, State, Message, Theme, Renderer, Widget>
    View<'a, State, Message, Theme, Renderer> for T
where
    T: Fn(&'a State) -> Widget,
    State: 'static,
    Widget: Into<Element<'a, Message, Theme, Renderer>>,
{
    fn view(
        &self,
        state: &'a State,
    ) -> impl Into<Element<'a, Message, Theme, Renderer>> {
        self(state)
    }
}

/// The renderer of some [`Program`].
pub trait Renderer: text::Renderer + compositor::Default {}

impl<T> Renderer for T where T: text::Renderer + compositor::Default {}
