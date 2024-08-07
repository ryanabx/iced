//! Build interactive cross-platform applications.
use crate::core::text;
use crate::graphics::compositor;
use crate::shell::application;
use crate::{Element, Executor, Settings, Subscription, Task};

pub use application::{Appearance, DefaultStyle};

/// An interactive cross-platform application.
///
/// This trait is the main entrypoint of Iced. Once implemented, you can run
/// your GUI application by simply calling [`run`](#method.run).
///
/// - On native platforms, it will run in its own window.
/// - On the web, it will take control of the `<title>` and the `<body>` of the
///   document.
///
/// An [`Application`] can execute asynchronous actions by returning a
/// [`Task`] in some of its methods.
///
/// When using an [`Application`] with the `debug` feature enabled, a debug view
/// can be toggled by pressing `F12`.
///
/// # Examples
/// [The repository has a bunch of examples] that use the [`Application`] trait:
///
/// - [`download_progress`], a basic application that asynchronously downloads
/// a dummy file of 100 MB and tracks the download progress.
/// - [`events`], a log of native events displayed using a conditional
/// [`Subscription`].
/// - [`game_of_life`], an interactive version of the [Game of Life], invented
/// by [John Horton Conway].
/// - [`pokedex`], an application that displays a random Pokédex entry (sprite
/// included!) by using the [PokéAPI].
/// - [`stopwatch`], a watch with start/stop and reset buttons showcasing how
/// to listen to time.
/// - [`todos`], a todos tracker inspired by [TodoMVC].
///
/// [The repository has a bunch of examples]: https://github.com/iced-rs/iced/tree/0.12/examples
/// [`clock`]: https://github.com/iced-rs/iced/tree/0.12/examples/clock
/// [`download_progress`]: https://github.com/iced-rs/iced/tree/0.12/examples/download_progress
/// [`events`]: https://github.com/iced-rs/iced/tree/0.12/examples/events
/// [`game_of_life`]: https://github.com/iced-rs/iced/tree/0.12/examples/game_of_life
/// [`pokedex`]: https://github.com/iced-rs/iced/tree/0.12/examples/pokedex
/// [`solar_system`]: https://github.com/iced-rs/iced/tree/0.12/examples/solar_system
/// [`stopwatch`]: https://github.com/iced-rs/iced/tree/0.12/examples/stopwatch
/// [`todos`]: https://github.com/iced-rs/iced/tree/0.12/examples/todos
/// [`Sandbox`]: crate::Sandbox
/// [PokéAPI]: https://pokeapi.co/
/// [TodoMVC]: http://todomvc.com/
///
/// ## A simple "Hello, world!"
///
/// If you just want to get started, here is a simple [`Application`] that
/// says "Hello, world!":
///
/// ```no_run
/// use iced::advanced::Application;
/// use iced::executor;
/// use iced::{Task, Element, Settings, Theme, Renderer};
///
/// pub fn main() -> iced::Result {
///     Hello::run(Settings::default())
/// }
///
/// struct Hello;
///
/// impl Application for Hello {
///     type Executor = executor::Default;
///     type Flags = ();
///     type Message = ();
///     type Theme = Theme;
///     type Renderer = Renderer;
///
///     fn new(_flags: ()) -> (Hello, Task<Self::Message>) {
///         (Hello, Task::none())
///     }
///
///     fn title(&self) -> String {
///         String::from("A cool application")
///     }
///
///     fn update(&mut self, _message: Self::Message) -> Task<Self::Message> {
///         Task::none()
///     }
///
///     fn view(&self) -> Element<Self::Message> {
///         "Hello, world!".into()
///     }
/// }
/// ```
pub trait Application: Sized
where
    Self::Theme: DefaultStyle,
{
    /// The [`Executor`] that will run commands and subscriptions.
    ///
    /// The [default executor] can be a good starting point!
    ///
    /// [`Executor`]: Self::Executor
    /// [default executor]: crate::executor::Default
    type Executor: Executor;

    /// The type of __messages__ your [`Application`] will produce.
    type Message: std::fmt::Debug + Send + 'static;

    /// The theme of your [`Application`].
    type Theme: Default;

    /// The renderer of your [`Application`].
    type Renderer: text::Renderer + compositor::Default;

    /// The data needed to initialize your [`Application`].
    type Flags;

    /// Initializes the [`Application`] with the flags provided to
    /// [`run`] as part of the [`Settings`].
    ///
    /// Here is where you should return the initial state of your app.
    ///
    /// Additionally, you can return a [`Task`] if you need to perform some
    /// async action in the background on startup. This is useful if you want to
    /// load state from a file, perform an initial HTTP request, etc.
    ///
    /// [`run`]: Self::run
    fn new(flags: Self::Flags) -> (Self, Task<Self::Message>);

    /// Returns the current title of the [`Application`].
    ///
    /// This title can be dynamic! The runtime will automatically update the
    /// title of your application when necessary.
    fn title(&self) -> String;

    /// Handles a __message__ and updates the state of the [`Application`].
    ///
    /// This is where you define your __update logic__. All the __messages__,
    /// produced by either user interactions or commands, will be handled by
    /// this method.
    ///
    /// Any [`Task`] returned will be executed immediately in the background.
    fn update(&mut self, message: Self::Message) -> Task<Self::Message>;

    /// Returns the widgets to display in the [`Application`].
    ///
    /// These widgets can produce __messages__ based on user interaction.
    fn view(&self) -> Element<'_, Self::Message, Self::Theme, Self::Renderer>;

    /// Returns the current [`Theme`] of the [`Application`].
    ///
    /// [`Theme`]: Self::Theme
    fn theme(&self) -> Self::Theme {
        Self::Theme::default()
    }

    /// Returns the current [`Appearance`] of the [`Application`].
    fn style(&self, theme: &Self::Theme) -> Appearance {
        theme.default_style()
    }

    /// Returns the event [`Subscription`] for the current state of the
    /// application.
    ///
    /// A [`Subscription`] will be kept alive as long as you keep returning it,
    /// and the __messages__ produced will be handled by
    /// [`update`](#tymethod.update).
    ///
    /// By default, this method returns an empty [`Subscription`].
    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::none()
    }

    /// Returns the scale factor of the [`Application`].
    ///
    /// It can be used to dynamically control the size of the UI at runtime
    /// (i.e. zooming).
    ///
    /// For instance, a scale factor of `2.0` will make widgets twice as big,
    /// while a scale factor of `0.5` will shrink them to half their size.
    ///
    /// By default, it returns `1.0`.
    fn scale_factor(&self) -> f64 {
        1.0
    }

    /// Runs the [`Application`].
    ///
    /// On native platforms, this method will take control of the current thread
    /// until the [`Application`] exits.
    ///
    /// On the web platform, this method __will NOT return__ unless there is an
    /// [`Error`] during startup.
    ///
    /// [`Error`]: crate::Error
    fn run(settings: Settings<Self::Flags>) -> crate::Result
    where
        Self: 'static,
    {
        #[allow(clippy::needless_update)]
        let renderer_settings = crate::graphics::Settings {
            default_font: settings.default_font,
            default_text_size: settings.default_text_size,
            antialiasing: if settings.antialiasing {
                Some(crate::graphics::Antialiasing::MSAAx4)
            } else {
                None
            },
            ..crate::graphics::Settings::default()
        };

        Ok(crate::shell::application::run::<
            Instance<Self>,
            Self::Executor,
            <Self::Renderer as compositor::Default>::Compositor,
        >(settings.into(), renderer_settings)?)
    }
}

struct Instance<A>(A)
where
    A: Application,
    A::Theme: DefaultStyle;

impl<A> crate::runtime::Program for Instance<A>
where
    A: Application,
    A::Theme: DefaultStyle,
{
    type Message = A::Message;
    type Theme = A::Theme;
    type Renderer = A::Renderer;

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        self.0.update(message)
    }

    fn view(&self) -> Element<'_, Self::Message, Self::Theme, Self::Renderer> {
        self.0.view()
    }
}

impl<A> application::Application for Instance<A>
where
    A: Application,
    A::Theme: DefaultStyle,
{
    type Flags = A::Flags;

    fn new(flags: Self::Flags) -> (Self, Task<A::Message>) {
        let (app, command) = A::new(flags);

        (Instance(app), command)
    }

    fn title(&self) -> String {
        self.0.title()
    }

    fn theme(&self) -> A::Theme {
        self.0.theme()
    }

    fn style(&self, theme: &A::Theme) -> Appearance {
        self.0.style(theme)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        self.0.subscription()
    }

    fn scale_factor(&self) -> f64 {
        self.0.scale_factor()
    }
}
