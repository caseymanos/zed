//! Drop Zone demo for iOS drag and drop.
//!
//! This demo shows a colored area that accepts file drops.
//! It displays the names of dropped files and changes color during drag.

use super::{BACKGROUND, OVERLAY, SUBTEXT, SURFACE, TEXT, TEAL};
use crate::{
    div, hsla, prelude::*, px, rgb, App, Context, FileDropEvent, Point, Render, Window,
};
use std::path::PathBuf;

/// State for tracking dropped files and drag state.
pub struct DropZone {
    /// Files that have been dropped
    dropped_files: Vec<PathBuf>,
    /// Whether a drag is currently over the drop zone
    is_drag_over: bool,
    /// Current drag position
    drag_position: Option<Point<crate::Pixels>>,
}

impl DropZone {
    pub fn new() -> Self {
        Self {
            dropped_files: Vec::new(),
            is_drag_over: false,
            drag_position: None,
        }
    }

    fn handle_file_drop(&mut self, event: FileDropEvent, cx: &mut Context<Self>) {
        match event {
            FileDropEvent::Entered { position, paths } => {
                log::info!("Drop entered at {:?} with {} paths", position, paths.0.len());
                self.is_drag_over = true;
                self.drag_position = Some(position);
                for path in paths.0.iter() {
                    log::info!("  Path: {:?}", path);
                }
                cx.notify();
            }
            FileDropEvent::Pending { position } => {
                self.drag_position = Some(position);
                cx.notify();
            }
            FileDropEvent::Submit { position } => {
                log::info!("Drop submitted at {:?}", position);
                self.drag_position = Some(position);
                cx.notify();
            }
            FileDropEvent::Exited => {
                log::info!("Drop exited");
                self.is_drag_over = false;
                self.drag_position = None;
                cx.notify();
            }
        }
    }

    pub fn render_with_back_button<F>(
        &self,
        _window: &mut Window,
        _on_back: F,
    ) -> impl IntoElement
    where
        F: Fn(&(), &mut Window, &mut App) + 'static,
    {
        let border_color = if self.is_drag_over {
            rgb(TEAL)
        } else {
            rgb(OVERLAY)
        };

        let bg_color = if self.is_drag_over {
            hsla(180.0 / 360.0, 0.5, 0.2, 0.5)
        } else {
            rgb(SURFACE).into()
        };

        let status_text = if self.is_drag_over {
            if let Some(pos) = self.drag_position {
                format!("Dragging at ({:.0}, {:.0})", pos.x.0, pos.y.0)
            } else {
                "Dragging...".to_string()
            }
        } else {
            "Drag files here from Files app".to_string()
        };

        // Clone dropped_files for the closure
        let dropped_files = self.dropped_files.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(BACKGROUND))
            .pt(px(100.0))
            .px_6()
            .gap_4()
            // Title
            .child(
                div()
                    .text_2xl()
                    .text_color(rgb(TEXT))
                    .child("Drop Zone Demo"),
            )
            .child(
                div()
                    .text_base()
                    .text_color(rgb(SUBTEXT))
                    .child("Test iOS drag and drop support"),
            )
            // Drop zone area
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .h(px(200.0))
                    .bg(bg_color)
                    .border_2()
                    .border_color(border_color)
                    .rounded_xl()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .text_color(if self.is_drag_over {
                                rgb(TEAL)
                            } else {
                                rgb(OVERLAY)
                            })
                            .child("+"),
                    )
                    .child(
                        div()
                            .text_base()
                            .text_color(if self.is_drag_over {
                                rgb(TEAL)
                            } else {
                                rgb(SUBTEXT)
                            })
                            .child(status_text),
                    ),
            )
            // Dropped files list
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(TEXT))
                            .child(format!("Dropped Files ({})", dropped_files.len())),
                    )
                    .children(dropped_files.iter().map(|path| {
                        let filename = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| path.display().to_string());
                        div()
                            .px_4()
                            .py_2()
                            .bg(rgb(SURFACE))
                            .rounded_lg()
                            .text_color(rgb(TEXT))
                            .child(filename)
                    })),
            )
            // Instructions
            .child(
                div()
                    .mt_4()
                    .p_4()
                    .bg(rgb(SURFACE))
                    .rounded_lg()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(SUBTEXT))
                            .child("To test: Open Files app in split view, drag a file here"),
                    ),
            )
    }
}

impl Render for DropZone {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Note: File drop events come through the platform layer and are dispatched
        // to the window's input callback. For now, just render the UI.
        // The drag_drop.rs module handles UIDropInteractionDelegate.
        self.render_with_back_button(window, |_, _, _| {})
    }
}
