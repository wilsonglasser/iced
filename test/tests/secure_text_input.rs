//! Regression tests for the fork's secure text input mirror.
use iced_test::core::keyboard;
use iced_test::core::keyboard::key::NativeCode;
use iced_test::core::keyboard::key::Physical;
use iced_test::core::mouse;
use iced_test::core::widget::Id;
use iced_test::core::Event;
use iced_test::core::Point;
use iced_test::Simulator;
use iced_widget::text_input;

fn input_bounds(sim: &mut Simulator<'_, String>) -> iced_test::core::Rectangle {
    sim.find(Id::new("field"))
        .expect("find input")
        .bounds()
}

fn focus(sim: &mut Simulator<'_, String>, bounds: iced_test::core::Rectangle) {
    let y = bounds.y + bounds.height / 2.0;

    sim.point_at(Point::new(bounds.x + 5.0, y));
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
}

fn select_all_by_double_click(sim: &mut Simulator<'_, String>, bounds: iced_test::core::Rectangle) {
    let y = bounds.y + bounds.height / 2.0;
    let middle = Point::new(bounds.x + 40.0, y);

    // First click focuses and places the caret.
    sim.point_at(middle);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);

    // Second click in quick succession registers as a double-click.
    sim.point_at(middle);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
}

fn select_all_by_drag(sim: &mut Simulator<'_, String>, bounds: iced_test::core::Rectangle) {
    let y = bounds.y + bounds.height / 2.0;

    sim.point_at(Point::new(bounds.x + 5.0, y));
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    sim.point_at(Point::new(bounds.x + bounds.width - 5.0, y));
    let _ = sim.simulate([Event::Mouse(mouse::Event::CursorMoved {
        position: Point::new(bounds.x + bounds.width - 5.0, y),
    })]);
    let _ = sim.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
}

fn new_sim() -> Simulator<'static, String> {
    let input = text_input("placeholder", "secret")
        .id("field")
        .secure(true)
        .on_input(|new_value| new_value);

    Simulator::new(input)
}

#[test]
fn double_click_select_backspace_fires_on_input() {
    let mut sim = new_sim();
    let bounds = input_bounds(&mut sim);

    select_all_by_double_click(&mut sim, bounds);

    let _ = sim.tap_key(keyboard::Key::Named(keyboard::key::Named::Backspace));

    let messages: Vec<String> = sim.into_messages().collect();

    assert!(
        messages.last().is_some_and(|message| message.is_empty()),
        "double-click select + backspace should publish an empty on_input, got: {messages:?}"
    );
}

#[test]
fn drag_select_backspace_fires_on_input() {
    let mut sim = new_sim();
    let bounds = input_bounds(&mut sim);

    select_all_by_drag(&mut sim, bounds);

    let _ = sim.tap_key(keyboard::Key::Named(keyboard::key::Named::Backspace));

    let messages: Vec<String> = sim.into_messages().collect();

    assert!(
        messages.last().is_some_and(|message| message.is_empty()),
        "drag-select + backspace should publish an empty on_input, got: {messages:?}"
    );
}

#[test]
fn ctrl_a_select_backspace_fires_on_input() {
    let mut sim = new_sim();
    let bounds = input_bounds(&mut sim);

    focus(&mut sim, bounds);

    let key = keyboard::Key::Character("a".into());
    let _ = sim.simulate([
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: Physical::Unidentified(NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::CTRL,
            text: None,
            repeat: false,
        }),
        Event::Keyboard(keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: Physical::Unidentified(NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::CTRL,
        }),
    ]);

    let _ = sim.tap_key(keyboard::Key::Named(keyboard::key::Named::Backspace));

    let messages: Vec<String> = sim.into_messages().collect();

    assert!(
        messages.last().is_some_and(|message| message.is_empty()),
        "ctrl+a + backspace should publish an empty on_input, got: {messages:?}"
    );
}
