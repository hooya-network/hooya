use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::Orientation;
use gtk::Widget;

const MASON_ROW_HEIGHT: i32 = 300;

#[derive(Default, Debug)]
pub struct MasonGridLayout {}

#[derive(Clone)]
struct GridItem {
    widget: Widget,
    height: i32,
    width: i32,
}

#[glib::object_subclass]
impl ObjectSubclass for MasonGridLayout {
    const NAME: &'static str = "MasonGridLayout";
    type Type = super::MasonGridLayout;
    type ParentType = gtk::LayoutManager;
}

impl ObjectImpl for MasonGridLayout {}
impl LayoutManagerImpl for MasonGridLayout {
    fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
        gtk::SizeRequestMode::HeightForWidth
    }

    fn measure(
        &self,
        widget: &gtk::Widget,
        orientation: gtk::Orientation,
        for_size: i32,
    ) -> (i32, i32, i32, i32) {
        let mut nat_size = 0;

        if for_size < 0 {
            // Eh, may not be the right way to handle this
            return (0, 0, -1, -1);
        }

        if orientation == Orientation::Vertical {
            let rows = self.derive_rows(widget, for_size);
            for r in rows {
                nat_size += r[0].height
            }
            (nat_size, nat_size, -1, -1)
        } else {
            // Also this is definitely wrong but this dimension doesn't matter
            // as we will always resize to fit the width
            (0, 0, -1, -1)
        }
    }

    fn allocate(
        &self,
        widget: &gtk::Widget,
        width: i32,
        _height: i32,
        _baseline: i32,
    ) {
        let (mut x_offset, mut y_offset) = (0, 0);
        let rows = self.derive_rows(widget, width);
        for r in &rows {
            for c in r {
                c.widget.size_allocate(
                    &gtk::Allocation::new(
                        x_offset, y_offset, c.width, c.height,
                    ),
                    -1,
                );
                x_offset += c.width;
            }
            x_offset = 0;
            // All items on a row are the same height
            y_offset += r[0].height;
        }
    }
}

impl MasonGridLayout {
    fn derive_rows(
        &self,
        widget: &gtk::Widget,
        width: i32,
    ) -> Vec<Vec<GridItem>> {
        let mut child = widget.first_child().unwrap();
        let mut child_width;
        let mut x_offset = 0;
        let mut curr_row: Vec<GridItem> = vec![];
        let mut rows = vec![];

        loop {
            if !child.should_layout() {
                continue;
            }

            let child_height = MASON_ROW_HEIGHT;
            child_width = scaled_width_given_height(&child, child_height);

            // Create a new row
            if !curr_row.is_empty() && x_offset + child_width > width {
                // Expand-to-fill all except last row
                let row_width = x_offset;
                let scaled_height = (width * MASON_ROW_HEIGHT) / (row_width);
                for c in curr_row.iter_mut() {
                    let dividend = width * c.width;
                    let scaled_width = if dividend % row_width == 0 {
                        dividend / row_width
                    } else {
                        (dividend / row_width) + 1
                    };
                    c.height = scaled_height;
                    c.width = scaled_width;
                }

                rows.push(curr_row.clone());

                // Start next row
                curr_row = vec![GridItem {
                    widget: child.clone(),
                    width: child_width,
                    height: child_height,
                }];
                x_offset = child_width;
            } else {
                x_offset += child_width;
                curr_row.push(GridItem {
                    widget: child.clone(),
                    width: child_width,
                    height: child_height,
                });
            }

            if let Some(next_child) = child.next_sibling() {
                child = next_child;
            } else {
                break;
            }
        }

        if !curr_row.is_empty() {
            rows.push(curr_row);
        }

        rows
    }
}

fn scaled_width_given_height(child: &Widget, height: i32) -> i32 {
    let (child_req, _) = child.preferred_size();
    let (child_min_width, child_nat_width, _, _) =
        child.measure(Orientation::Horizontal, -1);
    let (child_min_height, child_nat_height, _, _) =
        child.measure(Orientation::Vertical, -1);

    let mut child_width = child_min_width.max(child_req.width());
    let mut child_height = child_min_height.max(child_req.height());

    if child_req.height() == 0 {
        child_height = child_min_height.max(child_nat_height);
    }
    if child_req.width() == 0 {
        child_width = child_min_width.max(child_nat_width);
    }

    height * child_width / child_height
}
