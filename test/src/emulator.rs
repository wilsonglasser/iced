//! Run your application in a headless runtime.
use crate::core;
use crate::core::font;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::shell;
use crate::core::time::Instant;
use crate::core::widget;
use crate::core::window;
use crate::core::{Bytes, Element, Point, Size};
use crate::instruction;
use crate::program;
use crate::program::Program;
use crate::runtime;
use crate::runtime::futures::futures::StreamExt;
use crate::runtime::futures::futures::channel::mpsc;
use crate::runtime::futures::futures::stream;
use crate::runtime::futures::subscription;
use crate::runtime::futures::{Executor, Runtime};
use crate::runtime::task;
use crate::runtime::user_interface;
use crate::runtime::{Task, UserInterface};
use crate::{Instruction, Selector};

use std::borrow::Cow;
use std::fmt;

/// A headless runtime that can run iced applications and execute
/// [instructions](crate::Instruction).
///
/// An [`Emulator`] runs its program as faithfully as possible to the real thing.
/// It will run subscriptions and tasks with the [`Executor`](Program::Executor) of
/// the [`Program`].
///
/// If you want to run a simulation without side effects, use a [`Simulator`](crate::Simulator)
/// instead.
pub struct Emulator<P: Program> {
    state: P::State,
    runtime: Runtime<P::Executor, mpsc::Sender<Event<P>>, Event<P>>,
    renderer: P::Renderer,
    mode: Mode,
    size: Size,
    window: core::window::Id,
    cursor: mouse::Cursor,
    cache: Option<user_interface::Cache>,
    pending_tasks: usize,
    clipboard: Option<core::clipboard::Content>,
    clipboard_primary: Option<core::clipboard::Content>,
}

/// An emulation event.
pub enum Event<P: Program> {
    /// An action that must be [performed](Emulator::perform) by the [`Emulator`].
    Action(Action<P>),
    /// An [`Instruction`] failed to be executed.
    Failed(Instruction),
    /// The [`Emulator`] is ready.
    Ready,
}

/// An action that must be [performed](Emulator::perform) by the [`Emulator`].
pub struct Action<P: Program>(Action_<P>);

enum Action_<P: Program> {
    Runtime(runtime::Action<P::Message>),
    CountDown,
}

impl<P: Program + 'static> Emulator<P> {
    /// Creates a new [`Emulator`] of the [`Program`] with the given [`Mode`] and [`Size`].
    ///
    /// The [`Emulator`] will send [`Event`] notifications through the provided [`mpsc::Sender`].
    ///
    /// When the [`Emulator`] has finished booting, an [`Event::Ready`] will be produced.
    pub fn new(sender: mpsc::Sender<Event<P>>, program: &P, mode: Mode, size: Size) -> Emulator<P> {
        Self::with_preset(sender, program, mode, size, None)
    }

    /// Creates a new [`Emulator`] analogously to [`new`](Self::new), but it also takes a
    /// [`program::Preset`] that will be used as the initial state.
    ///
    /// When the [`Emulator`] has finished booting, an [`Event::Ready`] will be produced.
    pub fn with_preset(
        sender: mpsc::Sender<Event<P>>,
        program: &P,
        mode: Mode,
        size: Size,
        preset: Option<&program::Preset<P::State, P::Message>>,
    ) -> Emulator<P> {
        use renderer::Headless;

        let settings = program.settings();

        for font in &settings.fonts {
            load_font(font.clone()).expect("Font must be valid");
        }

        // TODO: Error handling
        let executor = P::Executor::new().expect("Create emulator executor");

        let backend = std::env::var("ICED_TEST_BACKEND").ok();

        let renderer = executor
            .block_on(P::Renderer::new(
                renderer::Settings::from(&settings),
                backend.as_deref(),
            ))
            .expect("Create emulator renderer");

        let runtime = Runtime::new(executor, sender);

        let (state, task) = runtime.enter(|| {
            if let Some(preset) = preset {
                preset.boot()
            } else {
                program.boot()
            }
        });

        let mut emulator = Self {
            state,
            runtime,
            renderer,
            mode,
            size,
            cursor: mouse::Cursor::Unavailable,
            window: core::window::Id::unique(),
            cache: Some(user_interface::Cache::default()),
            pending_tasks: 0,
            clipboard: None,
            clipboard_primary: None,
        };

        emulator.resubscribe(program);
        emulator.wait_for(task);

        emulator
    }

    /// Updates the state of the [`Emulator`] program.
    ///
    /// This is equivalent to calling the [`Program::update`] function,
    /// resubscribing to any subscriptions, and running the resulting tasks
    /// concurrently.
    pub fn update(&mut self, program: &P, message: P::Message) {
        let task = self
            .runtime
            .enter(|| program.update(&mut self.state, message));

        self.resubscribe(program);

        match self.mode {
            Mode::Zen if self.pending_tasks > 0 => self.wait_for(task),
            _ => {
                if let Some(stream) = task::into_stream(task) {
                    self.runtime.run(
                        stream
                            .map(Action_::Runtime)
                            .map(Action)
                            .map(Event::Action)
                            .boxed(),
                    );
                }
            }
        }
    }

    /// Performs an [`Action`].
    ///
    /// Whenever an [`Emulator`] sends an [`Event::Action`], this
    /// method must be called to proceed with the execution.
    pub fn perform(&mut self, program: &P, action: Action<P>) {
        match action.0 {
            Action_::CountDown => {
                if self.pending_tasks > 0 {
                    self.pending_tasks -= 1;

                    if self.pending_tasks == 0 {
                        self.runtime.send(Event::Ready);
                    }
                }
            }
            Action_::Runtime(action) => match action {
                runtime::Action::Output(message) => {
                    self.update(program, message);
                }
                runtime::Action::Widget(operation) => {
                    let mut user_interface = UserInterface::build(
                        program.view(&self.state, self.window),
                        self.size,
                        self.cache.take().unwrap(),
                        &mut self.renderer,
                    );

                    let mut operation = Some(operation);

                    while let Some(mut current) = operation.take() {
                        user_interface.operate(&self.renderer, &mut current);

                        match current.finish() {
                            widget::operation::Outcome::None => {}
                            widget::operation::Outcome::Some(()) => {}
                            widget::operation::Outcome::Chain(next) => {
                                operation = Some(next);
                            }
                        }
                    }

                    self.cache = Some(user_interface.into_cache());
                }
                runtime::Action::Clipboard(action) => {
                    use crate::runtime::clipboard;

                    match action {
                        clipboard::Action::Read {
                            clipboard_kind,
                            kind,
                            channel,
                        } => {
                            let _ = channel
                                .send(read_clipboard(self.slot(clipboard_kind).as_ref(), kind));
                        }
                        clipboard::Action::Write {
                            clipboard_kind,
                            content,
                            channel,
                        } => {
                            *self.slot_mut(clipboard_kind) = Some(content);
                            let _ = channel.send(Ok(()));
                        }
                    }
                }
                runtime::Action::Window(action) => {
                    use crate::runtime::window;

                    match action {
                        window::Action::Open(id, _settings, sender) => {
                            self.window = id;

                            let _ = sender.send(self.window);
                        }
                        window::Action::GetOldest(sender) | window::Action::GetLatest(sender) => {
                            let _ = sender.send(Some(self.window));
                        }
                        window::Action::GetSize(id, sender) if id == self.window => {
                            let _ = sender.send(self.size);
                        }
                        window::Action::GetMaximized(id, sender) if id == self.window => {
                            let _ = sender.send(false);
                        }
                        window::Action::GetMinimized(id, sender) if id == self.window => {
                            let _ = sender.send(None);
                        }
                        window::Action::GetPosition(id, sender) if id == self.window => {
                            let _ = sender.send(Some(Point::ORIGIN));
                        }
                        window::Action::GetScaleFactor(id, sender) if id == self.window => {
                            let _ = sender.send(1.0);
                        }
                        window::Action::GetMode(id, sender) if id == self.window => {
                            let _ = sender.send(core::window::Mode::Windowed);
                        }
                        _ => {
                            // Ignored
                        }
                    }
                }
                runtime::Action::System(action) => {
                    // TODO
                    dbg!(action);
                }
                runtime::Action::Font(action) => {
                    use crate::runtime::font;

                    match action {
                        font::Action::Load { bytes, channel } => {
                            let result = load_font(bytes);
                            let _ = channel.send(result);
                        }
                        font::Action::List { channel } => {
                            use std::collections::BTreeSet;

                            let font_system =
                                crate::renderer::graphics::text::font_system()
                                    .read()
                                    .expect("Read from font system");

                            let families =
                                BTreeSet::from_iter(font_system.families());

                            let _ = channel.send(Ok(families
                                .into_iter()
                                .map(core::font::Family::name)
                                .collect()));
                        }
                        // Changing the default font requires recreating
                        // the renderer (see the winit shell); no
                        // emulated program uses it so far.
                        font::Action::SetDefaults { .. } => {}
                    }
                }
                runtime::Action::Image(action) => {
                    // TODO
                    dbg!(action);
                }
                runtime::Action::Backend(action) => {
                    // TODO
                    dbg!(action);
                }
                runtime::Action::Event { window, event } => {
                    // TODO
                    dbg!(window, event);
                }
                runtime::Action::Tick => {
                    // TODO
                }
                runtime::Action::Exit => {
                    // TODO
                }
                runtime::Action::Reload => {
                    // TODO
                }
            },
        }
    }

    /// Runs an [`Instruction`].
    ///
    /// If the [`Instruction`] executes successfully, an [`Event::Ready`] will be
    /// produced by the [`Emulator`].
    ///
    /// Otherwise, an [`Event::Failed`] will be triggered.
    pub fn run(&mut self, program: &P, instruction: &Instruction) {
        let mut user_interface = UserInterface::build(
            program.view(&self.state, self.window),
            self.size,
            self.cache.take().unwrap(),
            &mut self.renderer,
        );

        let mut messages = shell::Bus::new();

        match instruction {
            Instruction::Interact(interaction) => {
                let Some(events) = interaction.events(|target| match target {
                    instruction::Target::Id(id) => {
                        use widget::Operation;

                        let mut operation = Selector::find(widget::Id::from(id.to_owned()));

                        user_interface.operate(
                            &self.renderer,
                            &mut widget::operation::black_box(&mut operation),
                        );

                        match operation.finish() {
                            widget::operation::Outcome::Some(widget) => {
                                Some(widget?.visible_bounds()?.center())
                            }
                            _ => None,
                        }
                    }
                    instruction::Target::Text(text) => {
                        use widget::Operation;

                        let mut operation = Selector::find(text.as_str());

                        user_interface.operate(
                            &self.renderer,
                            &mut widget::operation::black_box(&mut operation),
                        );

                        match operation.finish() {
                            widget::operation::Outcome::Some(text) => {
                                Some(text?.visible_bounds()?.center())
                            }
                            _ => None,
                        }
                    }
                    instruction::Target::Point(position) => Some(*position),
                }) else {
                    self.runtime.send(Event::Failed(instruction.clone()));
                    self.cache = Some(user_interface.into_cache());
                    return;
                };

                // Events are dispatched one at a time so widget-level
                // clipboard requests can be fulfilled from the emulated
                // clipboard *between* events. In the winit shell the read
                // completes asynchronously while later input (say, the key
                // release of a `ctrl+v` chord) hasn't been delivered yet;
                // fulfilling per event reproduces that ordering, a single
                // batched update would let the release cancel the pending
                // paste before the read result ever arrives.
                let mut statuses = Vec::with_capacity(events.len());

                for event in &events {
                    if let core::Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                        self.cursor = mouse::Cursor::Available(*position);
                    }

                    let (state, event_statuses) = user_interface.update(
                        &window::Headless,
                        &shell::Waker::noop(),
                        std::slice::from_ref(event),
                        self.cursor,
                        &mut self.renderer,
                        &mut messages,
                    );

                    statuses.extend(event_statuses);

                    // Fulfill widget-level clipboard requests (e.g. a paste
                    // in a `text_input`), feeding the results back as
                    // clipboard events, analogous to what the winit shell
                    // does with the system clipboard.
                    let mut clipboard_events = Vec::new();

                    if let user_interface::State::Updated {
                        clipboard: requests,
                        ..
                    } = state
                    {
                        for (clipboard_kind, kind) in requests.reads {
                            clipboard_events.push(core::Event::Clipboard(
                                core::clipboard::Event::Read(
                                    read_clipboard(self.slot(clipboard_kind).as_ref(), kind)
                                        .map(std::sync::Arc::new),
                                ),
                            ));
                        }

                        if let Some((clipboard_kind, content)) = requests.write {
                            // Field access, not `slot_mut`: the live view
                            // still borrows `self.state` here, so only a
                            // disjoint field borrow is allowed.
                            match clipboard_kind {
                                core::clipboard::ClipboardKind::Standard => {
                                    self.clipboard = Some(content);
                                }
                                core::clipboard::ClipboardKind::Primary => {
                                    self.clipboard_primary = Some(content);
                                }
                            }

                            clipboard_events.push(core::Event::Clipboard(
                                core::clipboard::Event::Written(Ok(())),
                            ));
                        }
                    }

                    if !clipboard_events.is_empty() {
                        let _ = user_interface.update(
                            &window::Headless,
                            &shell::Waker::noop(),
                            &clipboard_events,
                            self.cursor,
                            &mut self.renderer,
                            &mut messages,
                        );
                    }
                }

                self.cache = Some(user_interface.into_cache());

                // Broadcast the simulated events to the running subscriptions,
                // so global listeners (e.g. keyboard shortcut subscriptions or
                // `event::listen`) observe them like they would in the shell.
                for (event, status) in events.into_iter().zip(statuses) {
                    self.runtime.broadcast(subscription::Event::Interaction {
                        window: self.window,
                        event,
                        status,
                    });
                }

                let task = self.runtime.enter(|| {
                    Task::batch(
                        messages
                            .into_iter()
                            .map(|message| program.update(&mut self.state, message)),
                    )
                });

                self.resubscribe(program);
                self.wait_for(task);
            }
            Instruction::Expect(expectation) => match expectation {
                instruction::Expectation::Text(text) => {
                    use widget::Operation;

                    let mut operation = Selector::find(text.as_str());

                    user_interface.operate(
                        &self.renderer,
                        &mut widget::operation::black_box(&mut operation),
                    );

                    match operation.finish() {
                        widget::operation::Outcome::Some(Some(_text)) => {
                            self.runtime.send(Event::Ready);
                        }
                        _ => {
                            self.runtime.send(Event::Failed(instruction.clone()));
                        }
                    }

                    self.cache = Some(user_interface.into_cache());
                }
            },
        }
    }

    fn wait_for(&mut self, task: Task<P::Message>) {
        if let Some(stream) = task::into_stream(task) {
            match self.mode {
                Mode::Zen => {
                    self.pending_tasks += 1;

                    self.runtime.run(
                        stream
                            .map(Action_::Runtime)
                            .map(Action)
                            .map(Event::Action)
                            .chain(stream::once(async {
                                Event::Action(Action(Action_::CountDown))
                            }))
                            .boxed(),
                    );
                }
                Mode::Patient => {
                    self.runtime.run(
                        stream
                            .map(Action_::Runtime)
                            .map(Action)
                            .map(Event::Action)
                            .chain(stream::once(async { Event::Ready }))
                            .boxed(),
                    );
                }
                Mode::Immediate => {
                    self.runtime.run(
                        stream
                            .map(Action_::Runtime)
                            .map(Action)
                            .map(Event::Action)
                            .boxed(),
                    );
                    self.runtime.send(Event::Ready);
                }
            }
        } else if self.pending_tasks == 0 {
            self.runtime.send(Event::Ready);
        }
    }

    fn resubscribe(&mut self, program: &P) {
        self.runtime
            .track(subscription::into_recipes(self.runtime.enter(|| {
                program.subscription(&self.state).map(|message| {
                    Event::Action(Action(Action_::Runtime(runtime::Action::Output(message))))
                })
            })));
    }

    /// Returns the current view of the [`Emulator`].
    pub fn view(&self, program: &P) -> Element<'_, P::Message, P::Theme, P::Renderer> {
        program.view(&self.state, self.window)
    }

    /// Returns the current theme of the [`Emulator`].
    pub fn theme(&self, program: &P) -> Option<P::Theme> {
        program.theme(&self.state, self.window)
    }

    /// Takes a [`window::Screenshot`] of the current state of the [`Emulator`].
    pub fn screenshot(
        &mut self,
        program: &P,
        theme: &P::Theme,
        scale_factor: f32,
    ) -> window::Screenshot {
        use core::renderer::Headless;

        let style = program.style(&self.state, theme);

        let mut user_interface = UserInterface::build(
            program.view(&self.state, self.window),
            self.size,
            self.cache.take().unwrap(),
            &mut self.renderer,
        );

        // TODO: Nested redraws!
        let _ = user_interface.update(
            &window::Headless,
            &shell::Waker::noop(),
            &[core::Event::Window(window::Event::RedrawRequested(
                Instant::now(),
            ))],
            self.cursor,
            &mut self.renderer,
            &mut shell::Bus::new(),
        );

        // The shot is taken with the cursor where the emulator has put
        // it, which is the cursor every other dispatch already uses.
        // Anything the pointer decides at DRAW time (a hover style, a
        // drag preview, a highlighted drop target) is otherwise absent
        // from the picture while being present in the running program,
        // and a screenshot is the only way those are asserted at all.
        user_interface.draw(
            &mut self.renderer,
            theme,
            &renderer::Style {
                text_color: style.text_color,
            },
            self.cursor,
        );

        // Hand the widget-state cache back; taking it without restoring
        // would poison the next instruction with an unwrap on `None`.
        self.cache = Some(user_interface.into_cache());

        let physical_size = Size::new(
            (self.size.width * scale_factor).round() as u32,
            (self.size.height * scale_factor).round() as u32,
        );

        let rgba = self
            .renderer
            .screenshot(physical_size, scale_factor, style.background_color);

        window::Screenshot {
            rgba: Bytes::from(rgba),
            size: physical_size,
            scale_factor,
        }
    }

    /// Runs a widget [`Operation`](widget::Operation) over the current
    /// widget tree of the [`Emulator`].
    ///
    /// This is the emulator counterpart of
    /// [`UserInterface::operate`]: it lets tests and harnesses inspect
    /// or mutate widget state (query text, focus inputs, scroll
    /// scrollables) against the same widget-state cache the emulated
    /// interactions build up.
    pub fn operate(&mut self, program: &P, operation: &mut dyn widget::Operation) {
        let mut user_interface = UserInterface::build(
            program.view(&self.state, self.window),
            self.size,
            self.cache.take().unwrap(),
            &mut self.renderer,
        );

        user_interface.operate(&self.renderer, operation);

        self.cache = Some(user_interface.into_cache());
    }

    /// Returns the current contents of the emulated clipboard.
    ///
    /// An [`Emulator`] never touches the system clipboard; reads and
    /// writes, both the runtime tasks and the widget-level requests,
    /// are served from this in-memory value.
    pub fn clipboard(&self) -> Option<&core::clipboard::Content> {
        self.clipboard.as_ref()
    }

    /// Replaces the contents of the emulated clipboard.
    pub fn set_clipboard(&mut self, content: Option<core::clipboard::Content>) {
        self.clipboard = content;
    }

    /// Returns the current contents of the emulated primary clipboard
    /// (the X11 / Wayland selection).
    pub fn clipboard_primary(&self) -> Option<&core::clipboard::Content> {
        self.clipboard_primary.as_ref()
    }

    /// Replaces the contents of the emulated primary clipboard.
    pub fn set_clipboard_primary(&mut self, content: Option<core::clipboard::Content>) {
        self.clipboard_primary = content;
    }

    /// The emulated buffer backing the given clipboard kind. The two are
    /// kept apart so a test can't see a primary write as a standard one.
    fn slot(&self, kind: core::clipboard::ClipboardKind) -> &Option<core::clipboard::Content> {
        match kind {
            core::clipboard::ClipboardKind::Standard => &self.clipboard,
            core::clipboard::ClipboardKind::Primary => &self.clipboard_primary,
        }
    }

    fn slot_mut(
        &mut self,
        kind: core::clipboard::ClipboardKind,
    ) -> &mut Option<core::clipboard::Content> {
        match kind {
            core::clipboard::ClipboardKind::Standard => &mut self.clipboard,
            core::clipboard::ClipboardKind::Primary => &mut self.clipboard_primary,
        }
    }

    /// Returns a reference to the state of the [`Emulator`].
    pub fn state(&self) -> &P::State {
        &self.state
    }

    /// Turns the [`Emulator`] into its internal state.
    pub fn into_state(self) -> (P::State, core::window::Id) {
        (self.state, self.window)
    }
}

/// The strategy used by an [`Emulator`] when waiting for tasks to finish.
///
/// A [`Mode`] can be used to make an [`Emulator`] wait for side effects to finish before
/// continuing execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Waits for all tasks spawned by an [`Instruction`], as well as all tasks indirectly
    /// spawned by the the results of those tasks.
    ///
    /// This is the default.
    #[default]
    Zen,
    /// Waits only for the tasks directly spawned by an [`Instruction`].
    Patient,
    /// Never waits for any tasks to finish.
    Immediate,
}

impl Mode {
    /// A list of all the available modes.
    pub const ALL: &[Self] = &[Self::Zen, Self::Patient, Self::Immediate];
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zen => "Zen",
            Self::Patient => "Patient",
            Self::Immediate => "Immediate",
        })
    }
}

/// Serves a clipboard read request from the emulated clipboard
/// contents, honoring the requested [`Kind`](core::clipboard::Kind).
fn read_clipboard(
    content: Option<&core::clipboard::Content>,
    kind: core::clipboard::Kind,
) -> Result<core::clipboard::Content, core::clipboard::Error> {
    use core::clipboard::{Content, Error, Kind};

    match (content, kind) {
        (Some(content @ Content::Text(_)), Kind::Text)
        | (Some(content @ Content::Html(_)), Kind::Html)
        | (Some(content @ Content::Files(_)), Kind::Files) => Ok(content.clone()),
        _ => Err(Error::ContentNotAvailable),
    }
}

fn load_font(font: Cow<'static, [u8]>) -> Result<(), font::Error> {
    crate::renderer::graphics::text::font_system()
        .write()
        .expect("Write to font system")
        .load_font(font);

    Ok(())
}
