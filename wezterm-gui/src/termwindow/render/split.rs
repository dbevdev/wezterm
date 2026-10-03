use crate::termwindow::render::TripleLayerQuadAllocator;
use crate::termwindow::{UIItem, UIItemType};
use config::DimensionContext;
use mux::pane::Pane;
use mux::tab::{PositionedSplit, SplitDirection};
use std::sync::Arc;
use window::color::LinearRgba;

/// Returns the range of cells along a divider that borders a pane.
///
/// The divider spans `along_start..along_start + along_len` in the
/// direction of the line, and `across_start..across_start + thickness` in
/// the other direction. The pane spans `pane_along_start` with
/// `pane_along_len` along the line, and `pane_across_start` with
/// `pane_across_len` across it. Returns None unless the pane is directly on
/// one side of the divider and overlaps it along the line.
#[allow(clippy::too_many_arguments)]
fn bordering_segment(
    along_start: usize,
    along_len: usize,
    across_start: usize,
    thickness: usize,
    pane_along_start: usize,
    pane_along_len: usize,
    pane_across_start: usize,
    pane_across_len: usize,
) -> Option<(usize, usize)> {
    let adjacent = pane_across_start + pane_across_len == across_start
        || across_start + thickness == pane_across_start;
    if !adjacent {
        return None;
    }
    let start = along_start.max(pane_along_start);
    let end = (along_start + along_len).min(pane_along_start + pane_along_len);
    if end > start {
        Some((start, end - start))
    } else {
        None
    }
}

impl crate::TermWindow {
    /// Returns the thickness, in pixels, of the line drawn in the dividers
    fn pane_divider_line_width(&self) -> f32 {
        match &self.config.pane_divider_line_width {
            None => self.render_metrics.underline_height as f32,
            Some(width) => width
                .evaluate_as_pixels(DimensionContext {
                    dpi: self.dimensions.dpi as f32,
                    pixel_max: self.render_metrics.cell_size.width as f32,
                    pixel_cell: self.render_metrics.cell_size.width as f32,
                })
                .max(0.),
        }
    }

    /// Paints a divider, and the parts of it that border the active pane,
    /// whose cell rectangle (left, top, width, height) is `active_pane`,
    /// in the active_pane_split_color if that is configured.
    pub fn paint_split(
        &mut self,
        layers: &mut TripleLayerQuadAllocator,
        split: &PositionedSplit,
        pane: &Arc<dyn Pane>,
        active_pane: Option<(usize, usize, usize, usize)>,
    ) -> anyhow::Result<()> {
        let palette = pane.palette();
        let foreground = palette.split.to_linear();
        let highlight = self
            .config
            .active_pane_split_color
            .map(|color| color.to_linear());
        let cell_width = self.render_metrics.cell_size.width as f32;
        let cell_height = self.render_metrics.cell_size.height as f32;
        let line_width = self.pane_divider_line_width();

        let border = self.get_os_border();
        let first_row_offset = if self.show_tab_bar && !self.config.tab_bar_at_bottom {
            self.tab_bar_pixel_height()?
        } else {
            0.
        } + border.top.get() as f32;

        let (padding_left, padding_top) = self.padding_left_top();
        let origin_x = padding_left + border.left.get() as f32;
        let origin_y = first_row_offset + padding_top;

        // The line is drawn through the center of the divider, and extends
        // into the dividers at either end so that it meets any crossing lines.
        let dividers = self.pane_dividers();
        let thickness = split.thickness.max(1);

        // Draws the line along `len` cells of the divider starting at cell `start`
        let mut draw = |this: &mut Self, start: usize, len: usize, color: LinearRgba| {
            if line_width <= 0. {
                return Ok(());
            }
            let rect = if split.direction == SplitDirection::Horizontal {
                let cross = dividers.rows as f32 * cell_height;
                euclid::rect(
                    origin_x + split.left as f32 * cell_width + thickness as f32 * cell_width / 2.0,
                    origin_y + start as f32 * cell_height - (cross / 2.0),
                    line_width,
                    (dividers.rows as f32 + len as f32) * cell_height,
                )
            } else {
                let cross = dividers.cols as f32 * cell_width;
                euclid::rect(
                    origin_x + start as f32 * cell_width - (cross / 2.0),
                    origin_y
                        + split.top as f32 * cell_height
                        + thickness as f32 * cell_height / 2.0,
                    (dividers.cols as f32 + len as f32) * cell_width,
                    line_width,
                )
            };
            this.filled_rectangle(layers, 2, rect, color).map(|_| ())
        };

        let along_start = if split.direction == SplitDirection::Horizontal {
            split.top
        } else {
            split.left
        };
        draw(self, along_start, split.size, foreground)?;

        if let (Some(highlight), Some((left, top, width, height))) = (highlight, active_pane) {
            let segment = if split.direction == SplitDirection::Horizontal {
                bordering_segment(
                    split.top, split.size, split.left, thickness, top, height, left, width,
                )
            } else {
                bordering_segment(
                    split.left, split.size, split.top, thickness, left, width, top, height,
                )
            };
            if let Some((start, len)) = segment {
                draw(self, start, len, highlight)?;
            }
        }

        // The whole divider can be dragged to resize the panes
        let item_x =
            border.left.get() as usize + padding_left as usize + split.left * cell_width as usize;
        let item_y =
            padding_top as usize + first_row_offset as usize + split.top * cell_height as usize;
        self.ui_items
            .push(if split.direction == SplitDirection::Horizontal {
                UIItem {
                    x: item_x,
                    width: thickness * cell_width as usize,
                    y: item_y,
                    height: split.size * cell_height as usize,
                    item_type: UIItemType::Split(split.clone()),
                }
            } else {
                UIItem {
                    x: item_x,
                    width: split.size * cell_width as usize,
                    y: item_y,
                    height: thickness * cell_height as usize,
                    item_type: UIItemType::Split(split.clone()),
                }
            });

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::bordering_segment;

    #[test]
    fn bordering_segments() {
        // A vertical divider 3 cells wide at column 37, rows 0..24
        let divider = (0, 24, 37, 3);
        let segment = |pane_top, pane_height, pane_left, pane_width| {
            bordering_segment(
                divider.0,
                divider.1,
                divider.2,
                divider.3,
                pane_top,
                pane_height,
                pane_left,
                pane_width,
            )
        };
        // a pane on the left, top half
        assert_eq!(segment(0, 10, 0, 37), Some((0, 10)));
        // a pane on the right, bottom part
        assert_eq!(segment(12, 12, 40, 40), Some((12, 12)));
        // a pane that doesn't touch the divider
        assert_eq!(segment(0, 24, 0, 30), None);
        assert_eq!(segment(0, 24, 41, 39), None);
        // a pane that is adjacent but outside of the divider's extent
        assert_eq!(bordering_segment(0, 10, 37, 3, 12, 12, 0, 37), None);
    }
}
