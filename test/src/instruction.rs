//! A step in an end-to-end test.
use crate::core::keyboard;
use crate::core::mouse;
use crate::core::{Event, Point, SmolStr};
use crate::simulator;

use std::fmt;

/// A step in an end-to-end test.
///
/// An [`Instruction`] can be run by an [`Emulator`](crate::Emulator).
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    /// A user [`Interaction`].
    Interact(Interaction),
    /// A testing [`Expectation`].
    Expect(Expectation),
}

impl Instruction {
    /// Parses an [`Instruction`] from its textual representation.
    pub fn parse(line: &str) -> Result<Self, ParseError> {
        parser::run(line)
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Instruction::Interact(interaction) => interaction.fmt(f),
            Instruction::Expect(expectation) => expectation.fmt(f),
        }
    }
}

/// A user interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Interaction {
    /// A mouse interaction.
    Mouse(Mouse),
    /// A keyboard interaction.
    Keyboard(Keyboard),
}

impl Interaction {
    /// Creates an [`Interaction`] from a runtime [`Event`].
    ///
    /// This can be useful for recording tests during real usage.
    pub fn from_event(event: &Event) -> Option<Self> {
        Some(match event {
            Event::Mouse(mouse) => Self::Mouse(match mouse {
                mouse::Event::CursorMoved { position } => Mouse::Move(Target::Point(*position)),
                mouse::Event::ButtonPressed(button) => Mouse::Press {
                    button: *button,
                    target: None,
                },
                mouse::Event::ButtonReleased(button) => Mouse::Release {
                    button: *button,
                    target: None,
                },
                mouse::Event::WheelScrolled { delta } => Mouse::Scroll {
                    delta: *delta,
                    target: None,
                },
                _ => None?,
            }),
            Event::Keyboard(keyboard) => Self::Keyboard(match keyboard {
                keyboard::Event::KeyPressed {
                    key,
                    text,
                    modifiers,
                    ..
                } => {
                    // A command-modified press is a chord, not text input;
                    // record it as a `Shortcut` so it round-trips through
                    // the `ctrl+`/`alt+`/`logo+` syntax.
                    if modifiers.command() | modifiers.alt() {
                        Keyboard::Shortcut {
                            modifiers: *modifiers,
                            key: match key {
                                keyboard::Key::Named(keyboard::key::Named::Enter) => Key::Enter,
                                keyboard::Key::Named(keyboard::key::Named::Escape) => Key::Escape,
                                keyboard::Key::Named(keyboard::key::Named::Tab) => Key::Tab,
                                keyboard::Key::Named(keyboard::key::Named::Backspace) => {
                                    Key::Backspace
                                }
                                keyboard::Key::Character(c) => {
                                    let mut chars = c.chars();
                                    let first = chars.next()?;

                                    if chars.next().is_some() {
                                        None?;
                                    }

                                    Key::Char(first)
                                }
                                _ => None?,
                            },
                        }
                    } else {
                        match key {
                            keyboard::Key::Named(keyboard::key::Named::Enter) => {
                                Keyboard::Press(Key::Enter)
                            }
                            keyboard::Key::Named(keyboard::key::Named::Escape) => {
                                Keyboard::Press(Key::Escape)
                            }
                            keyboard::Key::Named(keyboard::key::Named::Tab) => {
                                Keyboard::Press(Key::Tab)
                            }
                            keyboard::Key::Named(keyboard::key::Named::Backspace) => {
                                Keyboard::Press(Key::Backspace)
                            }
                            _ => Keyboard::Typewrite(text.as_ref()?.to_string()),
                        }
                    }
                }
                keyboard::Event::KeyReleased { key, modifiers, .. } => {
                    // The release half of a chord is already covered by the
                    // `Shortcut` recorded on the press.
                    if modifiers.command() | modifiers.alt() {
                        None?;
                    }

                    match key {
                        keyboard::Key::Named(keyboard::key::Named::Enter) => {
                            Keyboard::Release(Key::Enter)
                        }
                        keyboard::Key::Named(keyboard::key::Named::Escape) => {
                            Keyboard::Release(Key::Escape)
                        }
                        keyboard::Key::Named(keyboard::key::Named::Tab) => {
                            Keyboard::Release(Key::Tab)
                        }
                        keyboard::Key::Named(keyboard::key::Named::Backspace) => {
                            Keyboard::Release(Key::Backspace)
                        }
                        _ => None?,
                    }
                }
                keyboard::Event::ModifiersChanged(_) => None?,
            }),
            _ => None?,
        })
    }

    /// Merges two interactions together, if possible.
    ///
    /// This method can turn certain sequences of interactions into a single one.
    /// For instance, a mouse movement, left button press, and left button release
    /// can all be merged into a single click interaction.
    ///
    /// Merging is lossy and, therefore, it is not always desirable if you are recording
    /// a test and want full reproducibility.
    ///
    /// If the interactions cannot be merged, the `next` interaction will be
    /// returned as the second element of the tuple.
    pub fn merge(self, next: Self) -> (Self, Option<Self>) {
        match (self, next) {
            (Self::Mouse(current), Self::Mouse(next)) => match (current, next) {
                (Mouse::Move(_), Mouse::Move(to)) => (Self::Mouse(Mouse::Move(to)), None),
                (
                    Mouse::Move(to),
                    Mouse::Press {
                        button,
                        target: None,
                    },
                ) => (
                    Self::Mouse(Mouse::Press {
                        button,
                        target: Some(to),
                    }),
                    None,
                ),
                (
                    Mouse::Move(to),
                    Mouse::Release {
                        button,
                        target: None,
                    },
                ) => (
                    Self::Mouse(Mouse::Release {
                        button,
                        target: Some(to),
                    }),
                    None,
                ),
                (
                    Mouse::Move(to),
                    Mouse::Scroll {
                        delta,
                        target: None,
                    },
                ) => (
                    Self::Mouse(Mouse::Scroll {
                        delta,
                        target: Some(to),
                    }),
                    None,
                ),
                (
                    Mouse::Scroll {
                        delta: current,
                        target: current_at,
                    },
                    Mouse::Scroll {
                        delta: next,
                        target: next_at,
                    },
                ) if (next_at.is_none() || next_at == current_at)
                    && merge_scroll_deltas(current, next).is_some() =>
                {
                    (
                        Self::Mouse(Mouse::Scroll {
                            delta: merge_scroll_deltas(current, next)
                                .expect("scroll deltas are mergeable"),
                            target: current_at,
                        }),
                        None,
                    )
                }
                (
                    Mouse::Press {
                        button: press,
                        target: press_at,
                    },
                    Mouse::Release {
                        button: release,
                        target: release_at,
                    },
                ) if press == release
                    && release_at
                        .as_ref()
                        .is_none_or(|release_at| Some(release_at) == press_at.as_ref()) =>
                {
                    (
                        Self::Mouse(Mouse::Click {
                            button: press,
                            target: press_at,
                        }),
                        None,
                    )
                }
                (
                    Mouse::Press {
                        button,
                        target: Some(press_at),
                    },
                    Mouse::Move(move_at),
                ) if press_at == move_at => (
                    Self::Mouse(Mouse::Press {
                        button,
                        target: Some(press_at),
                    }),
                    None,
                ),
                (
                    Mouse::Click {
                        button,
                        target: Some(click_at),
                    },
                    Mouse::Move(move_at),
                ) if click_at == move_at => (
                    Self::Mouse(Mouse::Click {
                        button,
                        target: Some(click_at),
                    }),
                    None,
                ),
                (current, next) => (Self::Mouse(current), Some(Self::Mouse(next))),
            },
            (Self::Keyboard(current), Self::Keyboard(next)) => match (current, next) {
                (Keyboard::Typewrite(current), Keyboard::Typewrite(next)) => (
                    Self::Keyboard(Keyboard::Typewrite(format!("{current}{next}"))),
                    None,
                ),
                (Keyboard::Press(current), Keyboard::Release(next)) if current == next => {
                    (Self::Keyboard(Keyboard::Type(current)), None)
                }
                (current, next) => (Self::Keyboard(current), Some(Self::Keyboard(next))),
            },
            (current, next) => (current, Some(next)),
        }
    }

    /// Returns a list of runtime events representing the [`Interaction`].
    ///
    /// The `find_target` closure must convert a [`Target`] into its screen
    /// coordinates.
    pub fn events(&self, find_target: impl FnOnce(&Target) -> Option<Point>) -> Option<Vec<Event>> {
        let mouse_move_ = |to| Event::Mouse(mouse::Event::CursorMoved { position: to });

        let mouse_press = |button| Event::Mouse(mouse::Event::ButtonPressed(button));

        let mouse_release = |button| Event::Mouse(mouse::Event::ButtonReleased(button));

        let key_press = |key| simulator::press_key(key, None);

        let key_release = |key| simulator::release_key(key);

        Some(match self {
            Interaction::Mouse(mouse) => match mouse {
                Mouse::Move(to) => vec![mouse_move_(find_target(to)?)],
                Mouse::Press {
                    button,
                    target: Some(at),
                } => vec![mouse_move_(find_target(at)?), mouse_press(*button)],
                Mouse::Press {
                    button,
                    target: None,
                } => {
                    vec![mouse_press(*button)]
                }
                Mouse::Release {
                    button,
                    target: Some(at),
                } => {
                    vec![mouse_move_(find_target(at)?), mouse_release(*button)]
                }
                Mouse::Release {
                    button,
                    target: None,
                } => {
                    vec![mouse_release(*button)]
                }
                Mouse::Click {
                    button,
                    target: Some(at),
                } => {
                    vec![
                        mouse_move_(find_target(at)?),
                        mouse_press(*button),
                        mouse_release(*button),
                    ]
                }
                Mouse::Click {
                    button,
                    target: None,
                } => {
                    vec![mouse_press(*button), mouse_release(*button)]
                }
                Mouse::Scroll {
                    delta,
                    target: Some(at),
                } => {
                    vec![
                        mouse_move_(find_target(at)?),
                        Event::Mouse(mouse::Event::WheelScrolled { delta: *delta }),
                    ]
                }
                Mouse::Scroll {
                    delta,
                    target: None,
                } => {
                    vec![Event::Mouse(mouse::Event::WheelScrolled { delta: *delta })]
                }
            },
            Interaction::Keyboard(keyboard) => match keyboard {
                Keyboard::Press(key) => vec![key_press(*key)],
                Keyboard::Release(key) => vec![key_release(*key)],
                Keyboard::Type(key) => vec![key_press(*key), key_release(*key)],
                Keyboard::Typewrite(text) => simulator::typewrite(text).collect(),
                Keyboard::Shortcut { modifiers, key } => vec![
                    Event::Keyboard(keyboard::Event::ModifiersChanged(*modifiers)),
                    simulator::press_key_with_modifiers(*key, None, *modifiers),
                    simulator::release_key_with_modifiers(*key, *modifiers),
                    Event::Keyboard(keyboard::Event::ModifiersChanged(
                        keyboard::Modifiers::default(),
                    )),
                ],
            },
        })
    }
}

/// Sums two [`mouse::ScrollDelta`]s of the same unit, or returns
/// `None` when the units differ.
fn merge_scroll_deltas(
    current: mouse::ScrollDelta,
    next: mouse::ScrollDelta,
) -> Option<mouse::ScrollDelta> {
    match (current, next) {
        (mouse::ScrollDelta::Lines { x, y }, mouse::ScrollDelta::Lines { x: dx, y: dy }) => {
            Some(mouse::ScrollDelta::Lines {
                x: x + dx,
                y: y + dy,
            })
        }
        (mouse::ScrollDelta::Pixels { x, y }, mouse::ScrollDelta::Pixels { x: dx, y: dy }) => {
            Some(mouse::ScrollDelta::Pixels {
                x: x + dx,
                y: y + dy,
            })
        }
        _ => None,
    }
}

impl fmt::Display for Interaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Interaction::Mouse(mouse) => mouse.fmt(f),
            Interaction::Keyboard(keyboard) => keyboard.fmt(f),
        }
    }
}

/// A mouse interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Mouse {
    /// The mouse was moved.
    Move(Target),
    /// A button was pressed.
    Press {
        /// The button.
        button: mouse::Button,
        /// The location of the press.
        target: Option<Target>,
    },
    /// A button was released.
    Release {
        /// The button.
        button: mouse::Button,
        /// The location of the release.
        target: Option<Target>,
    },
    /// A button was clicked.
    Click {
        /// The button.
        button: mouse::Button,
        /// The location of the click.
        target: Option<Target>,
    },
    /// The mouse wheel was scrolled.
    Scroll {
        /// The scroll movement.
        delta: mouse::ScrollDelta,
        /// The location of the scroll.
        target: Option<Target>,
    },
}

impl fmt::Display for Mouse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mouse::Move(target) => {
                write!(f, "move {}", target)
            }
            Mouse::Press { button, target } => {
                write!(f, "press {}", format::button_at(*button, target.as_ref()))
            }
            Mouse::Release { button, target } => {
                write!(f, "release {}", format::button_at(*button, target.as_ref()))
            }
            Mouse::Click { button, target } => {
                write!(f, "click {}", format::button_at(*button, target.as_ref()))
            }
            Mouse::Scroll { delta, target } => {
                write!(f, "scroll {}", format::scroll_delta(*delta))?;

                if let Some(target) = target {
                    write!(f, " {target}")?;
                }

                Ok(())
            }
        }
    }
}

/// The target of an interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A widget with the given identifier.
    Id(String),
    /// A UI element containing the given text.
    Text(String),
    /// A specific point of the viewport.
    Point(Point),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => f.write_str(&format::id(id)),
            Self::Point(point) => f.write_str(&format::point(*point)),
            Self::Text(text) => f.write_str(&format::string(text)),
        }
    }
}

/// A keyboard interaction.
#[derive(Debug, Clone, PartialEq)]
pub enum Keyboard {
    /// A key was pressed.
    Press(Key),
    /// A key was released.
    Release(Key),
    /// A key was "typed" (press and released).
    Type(Key),
    /// A bunch of text was typed.
    Typewrite(String),
    /// A key was "typed" while holding some modifiers
    /// (e.g. `type ctrl+shift+f`).
    Shortcut {
        /// The modifiers held during the chord.
        modifiers: keyboard::Modifiers,
        /// The key typed while the modifiers were held.
        key: Key,
    },
}

impl fmt::Display for Keyboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Keyboard::Press(key) => {
                write!(f, "press {}", format::key(*key))
            }
            Keyboard::Release(key) => {
                write!(f, "release {}", format::key(*key))
            }
            Keyboard::Type(key) => {
                write!(f, "type {}", format::key(*key))
            }
            Keyboard::Typewrite(text) => {
                write!(f, "type \"{text}\"")
            }
            Keyboard::Shortcut { modifiers, key } => {
                write!(
                    f,
                    "type {}{}",
                    format::modifiers(*modifiers),
                    format::key(*key)
                )
            }
        }
    }
}

/// A keyboard key.
///
/// Only a small subset of keys is supported currently!
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Key {
    Enter,
    Escape,
    Tab,
    Backspace,
    /// A plain character key (e.g. the `k` of `ctrl+k`).
    Char(char),
}

impl From<Key> for keyboard::Key {
    fn from(key: Key) -> Self {
        match key {
            Key::Enter => Self::Named(keyboard::key::Named::Enter),
            Key::Escape => Self::Named(keyboard::key::Named::Escape),
            Key::Tab => Self::Named(keyboard::key::Named::Tab),
            Key::Backspace => Self::Named(keyboard::key::Named::Backspace),
            Key::Char(c) => Self::Character(SmolStr::new(c.to_string())),
        }
    }
}

mod format {
    use super::*;

    pub fn button_at(button: mouse::Button, at: Option<&Target>) -> String {
        let button = self::button(button);

        if let Some(at) = at {
            if button.is_empty() {
                at.to_string()
            } else {
                format!("{} {}", button, at)
            }
        } else {
            button.to_owned()
        }
    }

    pub fn button(button: mouse::Button) -> &'static str {
        match button {
            mouse::Button::Left => "",
            mouse::Button::Right => "right",
            mouse::Button::Middle => "middle",
            mouse::Button::Back => "back",
            mouse::Button::Forward => "forward",
            mouse::Button::Other(_) => "other",
        }
    }

    pub fn point(point: Point) -> String {
        format!("({:.2}, {:.2})", point.x, point.y)
    }

    pub fn scroll_delta(delta: mouse::ScrollDelta) -> String {
        match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                format!("({x:.2}, {y:.2})")
            }
            mouse::ScrollDelta::Pixels { x, y } => {
                format!("pixels ({x:.2}, {y:.2})")
            }
        }
    }

    pub fn key(key: Key) -> String {
        match key {
            Key::Enter => "enter".to_owned(),
            Key::Escape => "escape".to_owned(),
            Key::Tab => "tab".to_owned(),
            Key::Backspace => "backspace".to_owned(),
            Key::Char(c) => c.to_string(),
        }
    }

    pub fn modifiers(modifiers: keyboard::Modifiers) -> String {
        let mut chord = String::new();

        if modifiers.control() {
            chord.push_str("ctrl+");
        }
        if modifiers.shift() {
            chord.push_str("shift+");
        }
        if modifiers.alt() {
            chord.push_str("alt+");
        }
        if modifiers.logo() {
            chord.push_str("logo+");
        }

        chord
    }

    pub fn string(text: &str) -> String {
        format!("\"{}\"", text.escape_default())
    }

    pub fn id(id: &str) -> String {
        format!("#{id}")
    }
}

/// A testing assertion.
///
/// Expectations are instructions that verify the current state of
/// the user interface of an application.
#[derive(Debug, Clone, PartialEq)]
pub enum Expectation {
    /// Expect some element to contain some text.
    Text(String),
}

impl fmt::Display for Expectation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expectation::Text(text) => {
                write!(f, "expect {}", format::string(text))
            }
        }
    }
}

pub use parser::Error as ParseError;

mod parser {
    use super::*;

    use nom::branch::alt;
    use nom::bytes::complete::tag;
    use nom::bytes::{is_not, take_while_m_n};
    use nom::character::complete::{alphanumeric1, char, multispace0, multispace1, satisfy};
    use nom::combinator::{map, map_opt, map_res, opt, recognize, success, value, verify};
    use nom::error::ParseError;
    use nom::multi::{fold, many1_count};
    use nom::number::float;
    use nom::sequence::{delimited, preceded, separated_pair, terminated};
    use nom::{Finish, IResult, Parser};

    /// A parsing error.
    #[derive(Debug, Clone, thiserror::Error)]
    #[error("parse error: {0}")]
    pub struct Error(nom::error::Error<String>);

    pub fn run(input: &str) -> Result<Instruction, Error> {
        match instruction.parse_complete(input).finish() {
            Ok((_rest, instruction)) => Ok(instruction),
            Err(error) => Err(Error(error.cloned())),
        }
    }

    fn instruction(input: &str) -> IResult<&str, Instruction> {
        alt((
            map(interaction, Instruction::Interact),
            map(expectation, Instruction::Expect),
        ))
        .parse(input)
    }

    fn interaction(input: &str) -> IResult<&str, Interaction> {
        // Keyboard goes first: `press enter` must parse as a key press,
        // not as a left-button mouse press with junk left over (the
        // mouse `press` parser accepts a bare button).
        alt((
            map(keyboard, Interaction::Keyboard),
            map(mouse, Interaction::Mouse),
        ))
        .parse(input)
    }

    fn mouse(input: &str) -> IResult<&str, Mouse> {
        let mouse_move = preceded(tag("move "), target).map(Mouse::Move);

        alt((
            mouse_move,
            mouse_click,
            mouse_press,
            mouse_release,
            mouse_scroll,
        ))
        .parse(input)
    }

    fn mouse_scroll(input: &str) -> IResult<&str, Mouse> {
        let (input, _) = tag("scroll ")(input)?;
        let (input, pixels) = opt(tag("pixels ")).parse(input)?;
        let (input, Point { x, y }) = point(input)?;
        let (input, at) = opt(target).parse(input)?;

        let delta = if pixels.is_some() {
            mouse::ScrollDelta::Pixels { x, y }
        } else {
            mouse::ScrollDelta::Lines { x, y }
        };

        Ok((input, Mouse::Scroll { delta, target: at }))
    }

    fn mouse_click(input: &str) -> IResult<&str, Mouse> {
        let (input, _) = tag("click ")(input)?;
        let (input, (button, target)) = mouse_button_at(input)?;

        Ok((input, Mouse::Click { button, target }))
    }

    fn mouse_press(input: &str) -> IResult<&str, Mouse> {
        let (input, _) = tag("press ")(input)?;
        let (input, (button, target)) = mouse_button_at(input)?;

        Ok((input, Mouse::Press { button, target }))
    }

    fn mouse_release(input: &str) -> IResult<&str, Mouse> {
        let (input, _) = tag("release ")(input)?;
        let (input, (button, target)) = mouse_button_at(input)?;

        Ok((input, Mouse::Release { button, target }))
    }

    fn mouse_button_at(input: &str) -> IResult<&str, (mouse::Button, Option<Target>)> {
        let (input, button) = mouse_button(input)?;
        let (input, at) = opt(target).parse(input)?;

        Ok((input, (button, at)))
    }

    fn target(input: &str) -> IResult<&str, Target> {
        // Leading whitespace is skipped so a target can follow a
        // non-empty prefix (`click right "Text"`, `scroll (0, -3) #list`).
        preceded(
            multispace0,
            alt((
                id.map(String::from).map(Target::Id),
                string.map(Target::Text),
                point.map(Target::Point),
            )),
        )
        .parse(input)
    }

    fn mouse_button(input: &str) -> IResult<&str, mouse::Button> {
        // Every name the formatter emits parses back, or a recorded
        // interaction would fail to replay. `Other(n)` is the exception:
        // it formats without its number, so it never round-tripped.
        alt((
            tag("right").map(|_| mouse::Button::Right),
            tag("middle").map(|_| mouse::Button::Middle),
            tag("back").map(|_| mouse::Button::Back),
            tag("forward").map(|_| mouse::Button::Forward),
            success(mouse::Button::Left),
        ))
        .parse(input)
    }

    fn keyboard(input: &str) -> IResult<&str, Keyboard> {
        alt((
            map(preceded(tag("type "), string), Keyboard::Typewrite),
            map(preceded(tag("type "), chord), |(modifiers, key)| {
                Keyboard::Shortcut { modifiers, key }
            }),
            map(preceded(tag("type "), key), Keyboard::Type),
            map(preceded(tag("press "), key), Keyboard::Press),
            map(preceded(tag("release "), key), Keyboard::Release),
        ))
        .parse(input)
    }

    /// A modifier chord: one or more `ctrl+` / `shift+` / `alt+` /
    /// `logo+` prefixes followed by a key (`ctrl+k`, `ctrl+shift+f`,
    /// `alt+enter`).
    fn chord(input: &str) -> IResult<&str, (keyboard::Modifiers, Key)> {
        let modifier = terminated(
            alt((
                value(keyboard::Modifiers::CTRL, tag("ctrl")),
                value(keyboard::Modifiers::SHIFT, tag("shift")),
                value(keyboard::Modifiers::ALT, tag("alt")),
                value(keyboard::Modifiers::LOGO, alt((tag("logo"), tag("cmd")))),
            )),
            char('+'),
        );

        let (input, modifiers) = fold(1.., modifier, keyboard::Modifiers::default, |acc, m| {
            acc | m
        })
        .parse(input)?;

        let (input, key) = alt((key, map(chord_char, Key::Char))).parse(input)?;

        Ok((input, (modifiers, key)))
    }

    fn chord_char(input: &str) -> IResult<&str, char> {
        satisfy(|c| c.is_ascii_alphanumeric()).parse(input)
    }

    fn expectation(input: &str) -> IResult<&str, Expectation> {
        map(preceded(tag("expect "), string), |text| {
            Expectation::Text(text)
        })
        .parse(input)
    }

    fn key(input: &str) -> IResult<&str, Key> {
        alt((
            map(tag("enter"), |_| Key::Enter),
            map(tag("escape"), |_| Key::Escape),
            map(tag("tab"), |_| Key::Tab),
            map(tag("backspace"), |_| Key::Backspace),
        ))
        .parse(input)
    }

    fn id(input: &str) -> IResult<&str, &str> {
        preceded(
            char('#'),
            recognize(many1_count(alt((alphanumeric1, tag("_"), tag("-"))))),
        )
        .parse(input)
    }

    fn point(input: &str) -> IResult<&str, Point> {
        let comma = whitespace(char(','));

        map(
            delimited(
                char('('),
                separated_pair(float(), comma, float()),
                char(')'),
            ),
            |(x, y)| Point { x, y },
        )
        .parse(input)
    }

    pub fn whitespace<'a, O, E: ParseError<&'a str>, F>(
        inner: F,
    ) -> impl Parser<&'a str, Output = O, Error = E>
    where
        F: Parser<&'a str, Output = O, Error = E>,
    {
        delimited(multispace0, inner, multispace0)
    }

    // Taken from https://github.com/rust-bakery/nom/blob/51c3c4e44fa78a8a09b413419372b97b2cc2a787/examples/string.rs
    //
    // Copyright (c) 2014-2019 Geoffroy Couprie
    //
    // Permission is hereby granted, free of charge, to any person obtaining
    // a copy of this software and associated documentation files (the
    // "Software"), to deal in the Software without restriction, including
    // without limitation the rights to use, copy, modify, merge, publish,
    // distribute, sublicense, and/or sell copies of the Software, and to
    // permit persons to whom the Software is furnished to do so, subject to
    // the following conditions:
    //
    // The above copyright notice and this permission notice shall be
    // included in all copies or substantial portions of the Software.
    //
    // THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
    // EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
    // MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
    // NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE
    // LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
    // OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
    // WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
    fn string(input: &str) -> IResult<&str, String> {
        #[derive(Debug, Clone, Copy)]
        enum Fragment<'a> {
            Literal(&'a str),
            EscapedChar(char),
            EscapedWS,
        }

        fn fragment(input: &str) -> IResult<&str, Fragment<'_>> {
            alt((
                map(string_literal, Fragment::Literal),
                map(escaped_char, Fragment::EscapedChar),
                value(Fragment::EscapedWS, escaped_whitespace),
            ))
            .parse(input)
        }

        fn string_literal<'a, E: ParseError<&'a str>>(
            input: &'a str,
        ) -> IResult<&'a str, &'a str, E> {
            let not_quote_slash = is_not("\"\\");

            verify(not_quote_slash, |s: &str| !s.is_empty()).parse(input)
        }

        fn unicode(input: &str) -> IResult<&str, char> {
            let parse_hex = take_while_m_n(1, 6, |c: char| c.is_ascii_hexdigit());

            let parse_delimited_hex =
                preceded(char('u'), delimited(char('{'), parse_hex, char('}')));

            let parse_u32 = map_res(parse_delimited_hex, move |hex| u32::from_str_radix(hex, 16));

            map_opt(parse_u32, std::char::from_u32).parse(input)
        }

        fn escaped_char(input: &str) -> IResult<&str, char> {
            preceded(
                char('\\'),
                alt((
                    unicode,
                    value('\n', char('n')),
                    value('\r', char('r')),
                    value('\t', char('t')),
                    value('\u{08}', char('b')),
                    value('\u{0C}', char('f')),
                    value('\\', char('\\')),
                    value('/', char('/')),
                    value('"', char('"')),
                )),
            )
            .parse(input)
        }

        fn escaped_whitespace(input: &str) -> IResult<&str, &str> {
            preceded(char('\\'), multispace1).parse(input)
        }

        let build_string = fold(0.., fragment, String::new, |mut string, fragment| {
            match fragment {
                Fragment::Literal(s) => string.push_str(s),
                Fragment::EscapedChar(c) => string.push(c),
                Fragment::EscapedWS => {}
            }
            string
        });

        delimited(char('"'), build_string, char('"')).parse(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Instruction {
        Instruction::parse(line).unwrap_or_else(|error| panic!("failed to parse {line:?}: {error}"))
    }

    fn roundtrip(line: &str) {
        assert_eq!(
            parse(line).to_string(),
            line,
            "instruction should round-trip"
        );
    }

    #[test]
    fn it_parses_scrolls() {
        assert_eq!(
            parse("scroll (0.00, -3.00)"),
            Instruction::Interact(Interaction::Mouse(Mouse::Scroll {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: -3.0 },
                target: None,
            }))
        );

        assert_eq!(
            parse("scroll pixels (0.00, -120.00) \"Hosts\""),
            Instruction::Interact(Interaction::Mouse(Mouse::Scroll {
                delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -120.0 },
                target: Some(Target::Text("Hosts".to_owned())),
            }))
        );

        roundtrip("scroll (0.00, -3.00)");
        roundtrip("scroll pixels (10.00, -120.00) #host-list");
    }

    #[test]
    fn it_parses_shortcuts() {
        assert_eq!(
            parse("type ctrl+k"),
            Instruction::Interact(Interaction::Keyboard(Keyboard::Shortcut {
                modifiers: keyboard::Modifiers::CTRL,
                key: Key::Char('k'),
            }))
        );

        assert_eq!(
            parse("type ctrl+shift+f"),
            Instruction::Interact(Interaction::Keyboard(Keyboard::Shortcut {
                modifiers: keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT,
                key: Key::Char('f'),
            }))
        );

        assert_eq!(
            parse("type alt+enter"),
            Instruction::Interact(Interaction::Keyboard(Keyboard::Shortcut {
                modifiers: keyboard::Modifiers::ALT,
                key: Key::Enter,
            }))
        );

        roundtrip("type ctrl+k");
        roundtrip("type ctrl+shift+f");
        roundtrip("type alt+enter");
    }

    #[test]
    fn it_parses_key_presses_as_keyboard_interactions() {
        // `press enter` used to be swallowed by the mouse `press`
        // parser (bare left button + ignored junk).
        assert_eq!(
            parse("press enter"),
            Instruction::Interact(Interaction::Keyboard(Keyboard::Press(Key::Enter)))
        );

        assert_eq!(
            parse("release tab"),
            Instruction::Interact(Interaction::Keyboard(Keyboard::Release(Key::Tab)))
        );

        roundtrip("press enter");
        roundtrip("release tab");
    }

    #[test]
    fn it_parses_targets_after_buttons() {
        // The space between a named button and its target used to make
        // the target unparseable.
        assert_eq!(
            parse("click right \"Host\""),
            Instruction::Interact(Interaction::Mouse(Mouse::Click {
                button: mouse::Button::Right,
                target: Some(Target::Text("Host".to_owned())),
            }))
        );

        roundtrip("click right \"Host\"");
    }

    #[test]
    fn it_parses_every_button_the_formatter_emits() {
        assert_eq!(
            parse("click middle (10, 20)"),
            Instruction::Interact(Interaction::Mouse(Mouse::Click {
                button: mouse::Button::Middle,
                target: Some(Target::Point(Point::new(10.0, 20.0))),
            }))
        );

        roundtrip("click middle (10.00, 20.00)");
        roundtrip("click back \"Files\"");
        roundtrip("click forward \"Files\"");
    }

    #[test]
    fn it_merges_scrolls() {
        let (merged, rest) = Interaction::Mouse(Mouse::Scroll {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            target: None,
        })
        .merge(Interaction::Mouse(Mouse::Scroll {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -2.0 },
            target: None,
        }));

        assert_eq!(
            merged,
            Interaction::Mouse(Mouse::Scroll {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: -3.0 },
                target: None,
            })
        );
        assert!(rest.is_none());
    }
}
