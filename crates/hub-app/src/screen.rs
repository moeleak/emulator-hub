//! The Android surface owns input only after the user clicks it. Coordinates
//! are measured against the displayed image, including aspect-ratio letterboxing.
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, renderer,
    widget::{Tree, tree},
};
use iced::{Event, Length, Point, Rectangle, Size, keyboard, window};
use material_ui_rs::{Element, Theme};

#[derive(Debug, Clone)]
pub enum Input {
    Touch { x: i32, y: i32, down: bool },
    Key { key: String, down: bool },
    Text(String),
    Wheel { dx: i32, dy: i32 },
    ReleaseAll,
}

#[derive(Default)]
struct State {
    focused: bool,
    dragging: bool,
    last: (i32, i32),
}

pub struct Screen<'a, Message> {
    child: Element<'a, Message>,
    width: u32,
    height: u32,
    on_input: Box<dyn Fn(Input) -> Message + 'a>,
}

pub fn view<'a, Message: 'a>(
    handle: iced::widget::image::Handle,
    width: u32,
    height: u32,
    on_input: impl Fn(Input) -> Message + 'a,
) -> Element<'a, Message> {
    let child = iced::widget::container(
        iced::widget::image(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(iced::ContentFit::Contain),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(|_| iced::widget::container::Style {
        background: Some(iced::Color::BLACK.into()),
        ..Default::default()
    });
    Element::new(Screen {
        child: child.into(),
        width,
        height,
        on_input: Box::new(on_input),
    })
}

fn image_bounds(bounds: Rectangle, width: u32, height: u32) -> Rectangle {
    let scale = (bounds.width / width.max(1) as f32).min(bounds.height / height.max(1) as f32);
    let w = width as f32 * scale;
    let h = height as f32 * scale;
    Rectangle {
        x: bounds.x + (bounds.width - w) / 2.0,
        y: bounds.y + (bounds.height - h) / 2.0,
        width: w,
        height: h,
    }
}

fn map_position(point: Point, bounds: Rectangle, width: u32, height: u32) -> (i32, i32) {
    let x = ((point.x - bounds.x) / bounds.width.max(1.0) * width as f32).floor();
    let y = ((point.y - bounds.y) / bounds.height.max(1.0) * height as f32).floor();
    (
        x.clamp(0.0, width.saturating_sub(1) as f32) as i32,
        y.clamp(0.0, height.saturating_sub(1) as f32) as i32,
    )
}

fn key_name(key: &keyboard::Key) -> Option<String> {
    match key {
        keyboard::Key::Character(value) => Some(value.to_string()),
        keyboard::Key::Named(keyboard::key::Named::Super) => Some("Meta".into()),
        keyboard::Key::Named(named) => Some(format!("{named:?}")),
        keyboard::Key::Unidentified => None,
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for Screen<'_, Message> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.child)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.child));
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = image_bounds(layout.bounds(), self.width, self.height);
        let position = cursor
            .position()
            .map(|p| map_position(p, bounds, self.width, self.height));
        let over = cursor.is_over(bounds);
        let mut input = None;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if over => {
                state.focused = true;
                state.dragging = true;
                state.last = position.unwrap_or_default();
                input = Some(Input::Touch {
                    x: state.last.0,
                    y: state.last.1,
                    down: true,
                });
            }
            Event::Mouse(mouse::Event::ButtonPressed(_)) if !over && state.focused => {
                state.focused = false;
                state.dragging = false;
                shell.publish((self.on_input)(Input::ReleaseAll));
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                state.last = position.unwrap_or(state.last);
                input = Some(Input::Touch {
                    x: state.last.0,
                    y: state.last.1,
                    down: true,
                });
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                state.last = position.unwrap_or(state.last);
                input = Some(Input::Touch {
                    x: state.last.0,
                    y: state.last.1,
                    down: false,
                });
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if over => {
                let (dx, dy) = match delta {
                    mouse::ScrollDelta::Lines { x, y } => {
                        ((*x * 120.0) as i32, (*y * 120.0) as i32)
                    }
                    mouse::ScrollDelta::Pixels { x, y } => (*x as i32, *y as i32),
                };
                input = Some(Input::Wheel { dx, dy });
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) if state.focused => {
                input = key_name(key).map(|key| Input::Key { key, down: true });
            }
            Event::Keyboard(keyboard::Event::KeyReleased { key, .. }) if state.focused => {
                input = key_name(key).map(|key| Input::Key { key, down: false });
            }
            Event::InputMethod(iced::advanced::input_method::Event::Commit(text))
                if state.focused =>
            {
                input = Some(Input::Text(text.clone()));
            }
            Event::Window(window::Event::Unfocused) | Event::Mouse(mouse::Event::CursorLeft)
                if state.focused || state.dragging =>
            {
                state.dragging = false;
                if matches!(event, Event::Window(window::Event::Unfocused)) {
                    state.focused = false;
                }
                input = Some(Input::ReleaseAll);
            }
            _ => {}
        }
        if let Some(input) = input {
            shell.publish((self.on_input)(input));
            shell.capture_event();
        }
    }
    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(image_bounds(layout.bounds(), self.width, self.height)) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portrait_input_accounts_for_letterboxing_and_hidpi_layout() {
        let bounds = image_bounds(
            Rectangle {
                x: 100.0,
                y: 50.0,
                width: 1000.0,
                height: 600.0,
            },
            1080,
            1920,
        );
        assert_eq!(bounds.width, 337.5);
        assert_eq!(
            map_position(bounds.center(), bounds, 1080, 1920),
            (540, 960)
        );
        assert_eq!(
            map_position(Point::new(5000.0, -10.0), bounds, 1080, 1920),
            (1079, 0)
        );
    }
}
