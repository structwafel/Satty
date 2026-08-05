use std::f32::consts::PI;

use super::{Drawable, Tool, ToolUpdateResult, Tools};
use crate::{
    math::{self, Vec2D},
    sketch_board::{
        KeyEventMsg, MouseButton, MouseEventMsg, MouseEventType, SketchBoardInput,
        SketchBoardOutput,
    },
};
use anyhow::Result;
use femtovg::{Color, Paint, Path};
use relm4::adw::gdk::ModifierType;
use relm4::{Sender, gtk::gdk::Key};

#[derive(Debug, Clone)]
pub struct Crop {
    pos: Vec2D,
    size: Vec2D,
    // True only while the crop area is being interactively created or adjusted, i.e. between
    // begin_drag and end_drag. An existing crop is always applied to the rendered output (see
    // render_native_resolution), so releasing the mouse is the commit and there is no pending
    // state left for Enter to confirm. Staying "editing" after the drag would make Escape reset
    // the crop instead of running the configured escape actions, silently dropping the crop the
    // user just made.
    editing: bool,
}

#[derive(Default)]
pub struct CropTool {
    crop: Option<Crop>,
    action: Option<CropToolAction>,
    input_enabled: bool,
    sender: Option<Sender<SketchBoardInput>>,
}

impl Crop {
    const HANDLE_RADIUS: f32 = 5.0;
    const HANDLE_BORDER: f32 = 2.0;

    fn new(pos: Vec2D) -> Self {
        Self {
            pos,
            size: Vec2D::zero(),
            editing: true,
        }
    }

    fn draw_single_handle(
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
        center: Vec2D,
        scale: f32,
    ) {
        let mut path = Path::new();
        path.arc(
            center.x,
            center.y,
            Crop::HANDLE_RADIUS / scale,
            0.0,
            2.0 * PI,
            femtovg::Solidity::Solid,
        );

        let border_paint =
            Paint::color(Color::rgbf(0.9, 0.9, 0.9)).with_line_width(Crop::HANDLE_BORDER / scale);
        let fill_paint = Paint::color(Color::rgbaf(0.0, 0.0, 0.0, 0.4));

        canvas.fill_path(&path, &fill_paint);
        canvas.stroke_path(&path, &border_paint);
    }

    pub fn get_rectangle(&self) -> (Vec2D, Vec2D) {
        math::rect_ensure_positive_size(self.pos, self.size)
    }

    fn get_handle_pos(crop_pos: Vec2D, crop_size: Vec2D, handle: CropHandle) -> Vec2D {
        match handle {
            CropHandle::TopLeftCorner => crop_pos,
            CropHandle::TopEdge => crop_pos + Vec2D::new(crop_size.x / 2.0, 0.0),
            CropHandle::TopRightCorner => crop_pos + Vec2D::new(crop_size.x, 0.0),
            CropHandle::RightEdge => crop_pos + Vec2D::new(crop_size.x, crop_size.y / 2.0),
            CropHandle::BottomRightCorner => crop_pos + Vec2D::new(crop_size.x, crop_size.y),
            CropHandle::BottomEdge => crop_pos + Vec2D::new(crop_size.x / 2.0, crop_size.y),
            CropHandle::BottomLeftCorner => crop_pos + Vec2D::new(0.0, crop_size.y),
            CropHandle::LeftEdge => crop_pos + Vec2D::new(0.0, crop_size.y / 2.0),
        }
    }
    fn get_closest_handle(&self, mouse_pos: Vec2D) -> (CropHandle, f32) {
        let mut min_distance_squared = f32::MAX;
        let mut closest_handle = CropHandle::TopLeftCorner;
        for h in CropHandle::all() {
            let handle_pos = Self::get_handle_pos(self.pos, self.size, h);
            let distance_squared = (handle_pos - mouse_pos).norm2();
            if distance_squared < min_distance_squared {
                min_distance_squared = distance_squared;
                closest_handle = h;
            }
        }
        (closest_handle, min_distance_squared)
    }
    fn test_handle_hit(&self, mouse_pos: Vec2D, margin2: f32) -> Option<CropHandle> {
        const HANDLE_SIZE: f32 = Crop::HANDLE_RADIUS + Crop::HANDLE_BORDER;
        const HANDLE_SIZE2: f32 = HANDLE_SIZE * HANDLE_SIZE;
        let allowed_distance2 = HANDLE_SIZE2 + margin2;

        let (handle, distance2) = self.get_closest_handle(mouse_pos);
        if distance2 < allowed_distance2 {
            Some(handle)
        } else {
            None
        }
    }
}

impl Drawable for Crop {
    fn draw(
        &self,
        canvas: &mut femtovg::Canvas<femtovg::renderer::OpenGl>,
        _font: femtovg::FontId,
        bounds: (Vec2D, Vec2D),
    ) -> Result<()> {
        let size = self.size;
        let scale = canvas.transform().average_scale();

        let shadow_paint = Paint::color(Color::rgbaf(0.0, 0.0, 0.0, 0.5))
            .with_fill_rule(femtovg::FillRule::EvenOdd);
        let (img_tl, img_br) = bounds;
        let img_size = img_br - img_tl;
        let mut shadow_path = Path::new();
        shadow_path.rect(img_tl.x, img_tl.y, img_size.x, img_size.y);
        shadow_path.rect(self.pos.x, self.pos.y, size.x, size.y);

        let border_paint = Paint::color(Color::rgbf(0.1, 0.1, 0.1)).with_line_width(2.0);
        let mut border_path = Path::new();
        border_path.rect(self.pos.x, self.pos.y, size.x, size.y);

        canvas.save();
        canvas.fill_path(&shadow_path, &shadow_paint);
        canvas.stroke_path(&border_path, &border_paint);

        // Handles are drawn for as long as a crop exists: the area stays adjustable, so hiding
        // them once the drag ends would suggest the crop can no longer be changed.
        Self::draw_single_handle(canvas, self.pos, scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(size.x / 2.0, 0.0), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(size.x, 0.0), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(0.0, size.y / 2.0), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(0.0, size.y), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(size.x / 2.0, size.y), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(size.x, size.y), scale);
        Self::draw_single_handle(canvas, self.pos + Vec2D::new(size.x, size.y / 2.0), scale);

        canvas.restore();
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum CropHandle {
    TopLeftCorner,
    TopEdge,
    TopRightCorner,
    RightEdge,
    BottomRightCorner,
    BottomEdge,
    BottomLeftCorner,
    LeftEdge,
}

enum CropToolAction {
    NewCrop,
    DragHandle(DragHandleState),
    Move(MoveState),
}

struct DragHandleState {
    handle: CropHandle,
    top_left_start: Vec2D,
    bottom_right_start: Vec2D,
}

struct MoveState {
    start: Vec2D,
}

impl CropTool {
    pub fn get_crop(&self) -> Option<&Crop> {
        match &self.crop {
            Some(c) => Some(c),
            None => None,
        }
    }
}

impl CropHandle {
    fn all() -> [CropHandle; 8] {
        [
            CropHandle::TopLeftCorner,
            CropHandle::TopEdge,
            CropHandle::TopRightCorner,
            CropHandle::RightEdge,
            CropHandle::BottomRightCorner,
            CropHandle::BottomEdge,
            CropHandle::BottomLeftCorner,
            CropHandle::LeftEdge,
        ]
    }
}

impl CropTool {
    const HANDLE_MARGIN_IN_2: f32 = 15.0 * 15.0;
    const HANDLE_MARGIN_OUT: f32 = 40.0;

    fn test_inside_crop(&self, mouse_pos: Vec2D, margin: f32) -> bool {
        let crop = match &self.crop {
            Some(c) => c,
            None => return false,
        };

        let (mut min_x, mut max_x) = (crop.pos.x, crop.pos.x + crop.size.x);
        if min_x > max_x {
            (min_x, max_x) = (max_x, min_x);
        }
        min_x -= margin;
        max_x += margin;

        let (mut min_y, mut max_y) = (crop.pos.y, crop.pos.y + crop.size.y);
        if min_y > max_y {
            (min_y, max_y) = (max_y, min_y);
        }
        min_y -= margin;
        max_y += margin;

        min_x < mouse_pos.x && mouse_pos.x < max_x && min_y < mouse_pos.y && mouse_pos.y < max_y
    }

    fn apply_drag_handle_transformation(
        crop: &mut Crop,
        state: &DragHandleState,
        direction: Vec2D,
    ) {
        let mut tl = state.top_left_start;
        let mut br = state.bottom_right_start;

        // apply transformation
        match state.handle {
            CropHandle::TopLeftCorner => {
                tl += direction;
            }
            CropHandle::TopEdge => {
                tl += Vec2D::new(0.0, direction.y);
            }
            CropHandle::TopRightCorner => {
                tl += Vec2D::new(0.0, direction.y);
                br += Vec2D::new(direction.x, 0.0);
            }
            CropHandle::RightEdge => {
                br += Vec2D::new(direction.x, 0.0);
            }
            CropHandle::BottomRightCorner => {
                br += direction;
            }
            CropHandle::BottomEdge => {
                br += Vec2D::new(0.0, direction.y);
            }
            CropHandle::BottomLeftCorner => {
                tl += Vec2D::new(direction.x, 0.0);
                br += Vec2D::new(0.0, direction.y);
            }
            CropHandle::LeftEdge => {
                tl += Vec2D::new(direction.x, 0.0);
            }
        }

        // convert back and save
        crop.pos = tl;
        crop.size = br - tl;
    }

    fn emit_crop_dimensions_update(&self) {
        if let (Some(crop), Some(sender)) = (&self.crop, &self.sender) {
            let (_pos, size) = crop.get_rectangle();
            let width = size.x.round() as i32;
            let height = size.y.round() as i32;
            sender
                .send(SketchBoardInput::Output(
                    SketchBoardOutput::DimensionsUpdate(Some((width, height))),
                ))
                .ok();
        }
    }

    fn begin_drag(&mut self, pos: Vec2D) -> ToolUpdateResult {
        match &self.crop {
            None => {
                // No crop exists, create a new one
                self.crop = Some(Crop::new(pos));
                self.action = Some(CropToolAction::NewCrop);
            }
            Some(c) => {
                if let Some(handle) = c.test_handle_hit(pos, CropTool::HANDLE_MARGIN_IN_2) {
                    // Crop exists and we are near a handle, drag it
                    self.action = Some(CropToolAction::DragHandle(DragHandleState {
                        handle,
                        top_left_start: c.pos,
                        bottom_right_start: c.pos + c.size,
                    }));
                } else if self.test_inside_crop(pos, 0.0) {
                    // Crop exists and we are inside it, move it
                    self.action = Some(CropToolAction::Move(MoveState { start: c.pos }));
                } else if self.test_inside_crop(pos, CropTool::HANDLE_MARGIN_OUT) {
                    // Crop exists and we are near the edge, drag from the closest handle
                    let (handle, _) = c.get_closest_handle(pos);
                    self.action = Some(CropToolAction::DragHandle(DragHandleState {
                        handle,
                        top_left_start: c.pos,
                        bottom_right_start: c.pos + c.size,
                    }));
                } else {
                    // Crop exists, but we far outside from it, create a new one
                    self.crop = Some(Crop::new(pos));
                    self.action = Some(CropToolAction::NewCrop);
                }
            }
        }
        if let Some(c) = &mut self.crop {
            c.editing = true;
        }
        ToolUpdateResult::Redraw
    }

    fn update_drag(&mut self, direction: Vec2D) -> ToolUpdateResult {
        let crop = match &mut self.crop {
            Some(c) => c,
            None => return ToolUpdateResult::Unmodified,
        };

        let action = match &self.action {
            Some(a) => a,
            None => return ToolUpdateResult::Unmodified,
        };

        match action {
            CropToolAction::NewCrop => {
                crop.size = direction;
                self.emit_crop_dimensions_update();
                ToolUpdateResult::Redraw
            }
            CropToolAction::DragHandle(state) => {
                Self::apply_drag_handle_transformation(crop, state, direction);
                self.emit_crop_dimensions_update();
                ToolUpdateResult::Redraw
            }
            CropToolAction::Move(state) => {
                crop.pos = state.start + direction;
                ToolUpdateResult::Redraw
            }
        }
    }

    fn end_drag(&mut self, direction: Vec2D) -> ToolUpdateResult {
        let Some(crop) = &mut self.crop else {
            return ToolUpdateResult::Unmodified;
        };

        let Some(action) = &self.action else {
            return ToolUpdateResult::Unmodified;
        };

        match action {
            // crop never returns "commit" because nothing gets
            // committed to the drawables stack
            CropToolAction::NewCrop => {
                crop.size = direction;
                crop.editing = false;
                self.action = None;
                self.emit_crop_dimensions_update();
                ToolUpdateResult::Redraw
            }
            CropToolAction::DragHandle(state) => {
                Self::apply_drag_handle_transformation(crop, state, direction);
                crop.editing = false;
                self.action = None;
                self.emit_crop_dimensions_update();
                ToolUpdateResult::Redraw
            }
            CropToolAction::Move(state) => {
                crop.pos = state.start + direction;
                crop.editing = false;
                self.action = None;
                self.emit_crop_dimensions_update();
                ToolUpdateResult::Redraw
            }
        }
    }
}

impl Tool for CropTool {
    fn active(&self) -> bool {
        // A crop that exists is always applied, so the tool counts as active for as long as there
        // is one. This is what keeps the commit/dismiss buttons in the style toolbar usable after
        // the drag ended, which is the mouse-only way to reset a crop.
        self.crop.is_some()
    }

    fn input_enabled(&self) -> bool {
        self.input_enabled
    }

    fn set_input_enabled(&mut self, value: bool) {
        self.input_enabled = value;
    }

    fn get_tool_type(&self) -> super::Tools {
        Tools::Crop
    }

    fn handle_key_event(&mut self, event: KeyEventMsg) -> ToolUpdateResult {
        match event.key {
            // Only an in-progress drag is cancelled/confirmed here. Once the drag has ended the
            // crop is a finished result, so both keys fall through to the actions the user
            // configured for them (actions-on-escape / actions-on-enter) with the crop applied.
            // Note that when Escape does reset the crop it keeps swallowing the event, so it never
            // resets the crop and copies the uncropped image in one press.
            //FIXME: use if let guards as soon as they're stabilized (1.95)
            Key::Escape if self.crop.is_some() => {
                if self.crop.as_mut().unwrap().editing {
                    self.handle_dismissed()
                } else {
                    ToolUpdateResult::Unmodified
                }
            }
            //FIXME: use if let guards as soon as they're stabilized (1.95)
            Key::Return if self.crop.is_some() => {
                if self.crop.as_mut().unwrap().editing {
                    self.handle_deactivated()
                } else {
                    ToolUpdateResult::Unmodified
                }
            }
            _ => ToolUpdateResult::Unmodified,
        }
    }

    fn handle_mouse_event(&mut self, event: MouseEventMsg) -> ToolUpdateResult {
        let ctrl_pressed = event.modifier.intersects(ModifierType::CONTROL_MASK);
        match event.type_ {
            MouseEventType::Click if event.button == MouseButton::Primary && ctrl_pressed => {
                self.handle_deactivated()
            }
            MouseEventType::Click
                if event.button == MouseButton::Secondary
                    && ctrl_pressed
                    && self.crop.is_some() =>
            {
                self.handle_dismissed()
            }
            MouseEventType::BeginDrag if event.button == MouseButton::Primary && !ctrl_pressed => {
                self.begin_drag(event.pos)
            }
            MouseEventType::EndDrag if event.button == MouseButton::Primary && !ctrl_pressed => {
                self.end_drag(event.pos)
            }
            MouseEventType::UpdateDrag if event.button == MouseButton::Primary && !ctrl_pressed => {
                self.update_drag(event.pos)
            }
            _ => ToolUpdateResult::Unmodified,
        }
    }

    fn handle_activated(&mut self) -> ToolUpdateResult {
        // Re-selecting the tool must not put an existing crop back into editing state, or Escape
        // would silently reset it again.
        if self.crop.is_some() {
            return ToolUpdateResult::Redraw;
        }
        ToolUpdateResult::Unmodified
    }

    fn handle_deactivated(&mut self) -> ToolUpdateResult {
        if let Some(c) = &mut self.crop {
            c.editing = false;
        }
        self.action = None;
        ToolUpdateResult::Redraw
    }

    fn handle_dismissed(&mut self) -> ToolUpdateResult {
        self.crop = None;
        self.action = None;

        if let Some(sender) = &self.sender {
            sender
                .send(SketchBoardInput::Output(
                    SketchBoardOutput::DimensionsUpdate(None),
                ))
                .ok();
        }
        ToolUpdateResult::RedrawAndStopPropagation
    }

    fn get_drawable(&self) -> Option<&dyn Drawable> {
        // the reason we always return None is because we dont want this tool
        // to show up with the standard rendering mechanism. Instead it will always
        // be drawn separately by using `get_crop(&self)`
        None
    }

    fn set_sender(&mut self, sender: Sender<SketchBoardInput>) {
        self.sender = Some(sender);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse_event(type_: MouseEventType, button: MouseButton, pos: Vec2D) -> MouseEventMsg {
        MouseEventMsg {
            type_,
            button,
            modifier: ModifierType::empty(),
            screen_pos: pos,
            is_touchpad: false,
            pos,
            n_pressed: 1,
            release: false,
        }
    }

    fn key_event(key: Key) -> KeyEventMsg {
        KeyEventMsg::new(key, 0, ModifierType::empty())
    }

    /// Draw a crop area: press at `pos`, drag by `size`, release. Positions for drag updates are
    /// relative to the start of the drag, mirroring what the sketch board passes in.
    fn draw_crop(tool: &mut CropTool, pos: Vec2D, size: Vec2D) {
        tool.handle_mouse_event(mouse_event(
            MouseEventType::BeginDrag,
            MouseButton::Primary,
            pos,
        ));
        tool.handle_mouse_event(mouse_event(
            MouseEventType::UpdateDrag,
            MouseButton::Primary,
            size,
        ));
        tool.handle_mouse_event(mouse_event(
            MouseEventType::EndDrag,
            MouseButton::Primary,
            size,
        ));
    }

    #[test]
    fn releasing_the_mouse_finishes_the_crop() {
        let mut tool = CropTool::default();
        draw_crop(&mut tool, Vec2D::new(100.0, 100.0), Vec2D::new(50.0, 40.0));

        let (pos, size) = tool.get_crop().expect("crop exists").get_rectangle();
        assert_eq!((pos.x, pos.y), (100.0, 100.0));
        assert_eq!((size.x, size.y), (50.0, 40.0));
        assert!(!tool.get_crop().unwrap().editing);
        // The tool stays "active" so the toolbar commit/dismiss buttons remain usable.
        assert!(tool.active());
    }

    #[test]
    fn escape_after_drawing_keeps_the_crop_and_falls_through() {
        let mut tool = CropTool::default();
        draw_crop(&mut tool, Vec2D::new(10.0, 10.0), Vec2D::new(30.0, 30.0));

        let result = tool.handle_key_event(key_event(Key::Escape));

        // Unmodified lets the sketch board run the configured actions-on-escape, with the crop
        // still in place, instead of silently resetting it.
        assert!(matches!(result, ToolUpdateResult::Unmodified));
        assert!(tool.get_crop().is_some());
    }

    #[test]
    fn enter_after_drawing_keeps_the_crop_and_falls_through() {
        let mut tool = CropTool::default();
        draw_crop(&mut tool, Vec2D::new(10.0, 10.0), Vec2D::new(30.0, 30.0));

        let result = tool.handle_key_event(key_event(Key::Return));

        assert!(matches!(result, ToolUpdateResult::Unmodified));
        assert!(tool.get_crop().is_some());
    }

    #[test]
    fn escape_mid_drag_resets_the_crop_without_leaking_the_event() {
        let mut tool = CropTool::default();
        tool.handle_mouse_event(mouse_event(
            MouseEventType::BeginDrag,
            MouseButton::Primary,
            Vec2D::new(10.0, 10.0),
        ));

        let result = tool.handle_key_event(key_event(Key::Escape));

        assert!(tool.get_crop().is_none());
        // Stopping propagation is what keeps a crop-resetting Escape from also running
        // actions-on-escape, which would copy the uncropped image.
        assert!(matches!(result, ToolUpdateResult::RedrawAndStopPropagation));
    }

    #[test]
    fn reselecting_the_tool_does_not_reopen_editing() {
        let mut tool = CropTool::default();
        draw_crop(&mut tool, Vec2D::new(10.0, 10.0), Vec2D::new(30.0, 30.0));

        tool.handle_activated();
        let result = tool.handle_key_event(key_event(Key::Escape));

        assert!(matches!(result, ToolUpdateResult::Unmodified));
        assert!(tool.get_crop().is_some());
    }

    #[test]
    fn ctrl_right_click_resets_a_finished_crop() {
        let mut tool = CropTool::default();
        draw_crop(&mut tool, Vec2D::new(10.0, 10.0), Vec2D::new(30.0, 30.0));

        let mut event = mouse_event(
            MouseEventType::Click,
            MouseButton::Secondary,
            Vec2D::new(20.0, 20.0),
        );
        event.modifier = ModifierType::CONTROL_MASK;
        let result = tool.handle_mouse_event(event);

        assert!(tool.get_crop().is_none());
        assert!(matches!(result, ToolUpdateResult::RedrawAndStopPropagation));
    }

    #[test]
    fn ctrl_right_click_without_a_crop_falls_through() {
        let mut tool = CropTool::default();

        let mut event = mouse_event(
            MouseEventType::Click,
            MouseButton::Secondary,
            Vec2D::new(20.0, 20.0),
        );
        event.modifier = ModifierType::CONTROL_MASK;
        let result = tool.handle_mouse_event(event);

        // Otherwise the crop tool would swallow actions-on-right-click.
        assert!(matches!(result, ToolUpdateResult::Unmodified));
    }

    #[test]
    fn adjusting_a_handle_finishes_editing_again() {
        let mut tool = CropTool::default();
        draw_crop(
            &mut tool,
            Vec2D::new(100.0, 100.0),
            Vec2D::new(100.0, 100.0),
        );

        // Grab the bottom right corner and drag it further out.
        tool.handle_mouse_event(mouse_event(
            MouseEventType::BeginDrag,
            MouseButton::Primary,
            Vec2D::new(200.0, 200.0),
        ));
        assert!(tool.get_crop().unwrap().editing);
        tool.handle_mouse_event(mouse_event(
            MouseEventType::EndDrag,
            MouseButton::Primary,
            Vec2D::new(50.0, 50.0),
        ));

        assert!(!tool.get_crop().unwrap().editing);
        let (pos, size) = tool.get_crop().unwrap().get_rectangle();
        assert_eq!((pos.x, pos.y), (100.0, 100.0));
        assert_eq!((size.x, size.y), (150.0, 150.0));
        assert!(matches!(
            tool.handle_key_event(key_event(Key::Escape)),
            ToolUpdateResult::Unmodified
        ));
    }
}
