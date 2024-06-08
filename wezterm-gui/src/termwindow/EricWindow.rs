
use std::ffi::{c_char, CStr};
include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::borrow::Cow;
use std::cell::{Ref, RefCell};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

use config::{Dimension, SrgbaTuple};
use mux::pane::Pane;
use mux::pane::Pattern::CaseInSensitiveString;
use termwiz::cell::CellAttributes;
use termwiz::color;
use termwiz::color::ColorSpec::TrueColor;
use termwiz::surface::Line;
use wezterm_term::{KeyCode, KeyModifiers, MouseEvent, StableRowIndex};
use window::color::LinearRgba;
use window::Modifiers;

use crate::termwindow::{DimensionContext, TermWindow};
use crate::termwindow::box_model::*;
use crate::termwindow::modal::Modal;
use crate::termwindow::render::corners::{
    BOTTOM_LEFT_ROUNDED_CORNER, BOTTOM_RIGHT_ROUNDED_CORNER, TOP_LEFT_ROUNDED_CORNER,
    TOP_RIGHT_ROUNDED_CORNER,
};
use crate::utilsprites::RenderMetrics;

pub struct EricRow {
    pub brief: Cow<'static, str>,
    pub rowIndex: StableRowIndex,
    pub first_y: usize,
}
pub struct EricWindow {
    element: RefCell<Option<Vec<ComputedElement>>>,
    selection: RefCell<String>,
    selected_row: RefCell<usize>,
    top_row: RefCell<StableRowIndex>,
    max_rows_on_screen: RefCell<usize>,
    commands: RefCell<Vec<(i64, EricRow)>>,
    row_indexes: RefCell<Vec<EricRow>>,
    results_dirty: RefCell<bool>,
    slab: * mut fzf_slab_t
}

impl EricWindow{
    pub fn new(term_window: &mut TermWindow) -> Self {
        unsafe {
            let slab = fzf_make_default_slab();
            Self {
                element: RefCell::new(None),
                selection: RefCell::new(String::new()),
                row_indexes: RefCell::new(Vec::new()),
                commands: RefCell::new(Vec::new()),
                selected_row: RefCell::new(0),
                top_row: RefCell::new(0),
                max_rows_on_screen: RefCell::new(0),
                results_dirty: RefCell::new(false),
                slab: slab
            }
        }
    }

    fn updated_input(&self) {
        *self.selected_row.borrow_mut() = 0;
        *self.top_row.borrow_mut() = 0;
    }

    fn move_up(&self) {
        let mut row = self.selected_row.borrow_mut();
        *row = row.saturating_sub(1);
        let mut top_row = self.top_row.borrow_mut();
        let commands = self.commands.borrow();
        *top_row = commands[*row].1.rowIndex;
    }

    fn move_down(&self) {
        let mut row = self.selected_row.borrow_mut();
        if(*row + 1 < self.commands.borrow().iter().count())
        {
            *row = row.saturating_add(1);
            let mut top_row = self.top_row.borrow_mut();
            let commands = self.commands.borrow();
            if(*row < commands.iter().count())
            {
                *top_row = commands[*row].1.rowIndex;
            }
        }
    }

    fn create_prompt_element(
        &self,
        term_window: &TermWindow,
        panel_width: f32,
        background_color: LinearRgba
    ) -> Element {
        let selection = self.selection.borrow();
        let selection = selection.as_str();
        let font = term_window
            .fonts
            .default_font()
            .expect("to resolve font");

        let prompt_elements =
            vec![
                Element::new(&font, ElementContent::Text(format!("> {selection}_")))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: term_window
                            .config
                            .command_palette_fg_color
                            .to_linear()
                            .into(),
                    })
                    .display(DisplayType::Block),
            ];
        self.create_panel_element(
            term_window,
            panel_width,
            1.0,
            background_color,
            BorderColor::new(
                    term_window.config.command_palette_fg_color.to_linear().into(),
                ),
            ElementContent::Children(prompt_elements),
        )
    }

    fn create_panel_element(
        &self,
        term_window: &TermWindow,
        panel_width: f32,
        panel_height: f32,
        background_color: LinearRgba,
        border_color: BorderColor,
        content: ElementContent
    ) -> Element {
        let selection = self.selection.borrow();
        let selection = selection.as_str();
        let font = term_window
            .fonts
            .default_font()
            .expect("to resolve font");

        let prompt_elements =
            vec![
                Element::new(&font, ElementContent::Text(format!("> {selection}_")))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg: LinearRgba::TRANSPARENT.into(),
                        text: term_window
                            .config
                            .command_palette_fg_color
                            .to_linear()
                            .into(),
                    })
                    .display(DisplayType::Block),
            ];
        Element::new(&font, content)
            .colors(ElementColors {
                border: BorderColor::default(),
                bg: background_color.into(),
                text: term_window.config.command_palette_fg_color.to_linear().into(),
            })
            .colors(ElementColors {
                border: border_color,
                bg: term_window.config.command_palette_bg_color.to_linear().into(),
                text: term_window.config.command_palette_fg_color.to_linear().into(),
            })
            .margin(BoxDimension {
                left: Dimension::Cells(0.25),
                right: Dimension::Cells(0.25),
                top: Dimension::Cells(0.25),
                bottom: Dimension::Cells(0.25),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(0.25),
                right: Dimension::Cells(0.25),
                top: Dimension::Cells(0.25),
                bottom: Dimension::Cells(0.25),
            })
            .border(BoxDimension::new(Dimension::Pixels(2.0)))
            .border_corners(Some(Corners {
                top_left: SizedPoly {
                    width: Dimension::Cells(0.25),
                    height: Dimension::Cells(0.25),
                    poly: TOP_LEFT_ROUNDED_CORNER,
                },
                top_right: SizedPoly {
                    width: Dimension::Cells(0.25),
                    height: Dimension::Cells(0.25),
                    poly: TOP_RIGHT_ROUNDED_CORNER,
                },
                bottom_left: SizedPoly {
                    width: Dimension::Cells(0.25),
                    height: Dimension::Cells(0.25),
                    poly: BOTTOM_LEFT_ROUNDED_CORNER,
                },
                bottom_right: SizedPoly {
                    width: Dimension::Cells(0.25),
                    height: Dimension::Cells(0.25),
                    poly: BOTTOM_RIGHT_ROUNDED_CORNER,
                },
            }))
            .display(DisplayType::Block)
            .min_width(Some(Dimension::Pixels(panel_width)))
            .max_width(Some(Dimension::Pixels(panel_width)))
            .min_height(Some(Dimension::Pixels(panel_height)))
    }
}

impl Modal for EricWindow{
    fn mouse_event(&self, event: MouseEvent, term_window: &mut TermWindow) -> anyhow::Result<()> {
        Ok(())
    }

    fn key_down(&self, key: KeyCode, mods: Modifiers, term_window: &mut TermWindow) -> anyhow::Result<bool> {
        match (key, mods) {
            (KeyCode::Escape, KeyModifiers::NONE) | (KeyCode::Char('g'), KeyModifiers::CTRL) => {
                term_window.cancel_modal();
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                let mut row = self.selected_row.borrow_mut();
                *row = row.saturating_sub(1);
                let commands = self.commands.borrow();
                let y = commands[*row].1.rowIndex + 1;
                let x = commands[*row].1.first_y;

                term_window.cancel_modal();

                if let Some(pane) = term_window.get_active_pane_or_overlay() {
                    let mut replace_current = false;
                    if let Some(existing) = pane.downcast_ref::<crate::overlay::CopyOverlay>() {
                        let mut params = existing.get_params();
                        params.editing_search = false;
                        existing.apply_params(params);
                        replace_current = true;
                    } else {
                        let copy = crate::overlay::CopyOverlay::with_pane(
                            term_window,
                            &pane,
                            crate::overlay::CopyModeParams {
                                pattern: CaseInSensitiveString("".to_string()),
                                editing_search: false,
                            },
                        )?;
                        let actualCopy = copy.downcast_ref::<crate::overlay::CopyOverlay>();
                        actualCopy.unwrap().select_cell(x, y);

                        term_window.assign_overlay_for_pane(copy.pane_id(), copy);
                    }
                    term_window.pane_state(pane.pane_id())
                        .overlay
                        .as_mut()
                        .map(|overlay| {
                            overlay.key_table_state.activate(crate::termwindow::keyevent::KeyTableArgs {
                                name: "copy_mode",
                                timeout_milliseconds: None,
                                replace_current,
                                one_shot: false,
                                until_unknown: false,
                                prevent_fallback: false,
                            });
                        });
                }

            }
            (KeyCode::UpArrow, KeyModifiers::NONE) | (KeyCode::Char('p'), KeyModifiers::CTRL) => {
                self.move_up();
            }
            (KeyCode::DownArrow, KeyModifiers::NONE) | (KeyCode::Char('n'), KeyModifiers::CTRL) => {
                self.move_down();
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                // Backspace to edit the selection
                let mut selection = self.selection.borrow_mut();
                selection.pop();
                self.updated_input();
            }
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                {
                    let mut selection = self.selection.borrow_mut();
                    selection.push(c);
                }
                *self.results_dirty.borrow_mut() = true;
                self.updated_input();
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn computed_element(&self, term_window: &mut TermWindow) -> anyhow::Result<Ref<[ComputedElement]>> {
        let panes = term_window.get_panes_to_render();
        let mut cloned_pane = panes[0].clone();

        let selection = self.selection.borrow();
        let selection = selection.as_str();

        let font = term_window
            .fonts
            .default_font()
            .expect("to resolve font");

        let dimensions = term_window.dimensions;
        let size = term_window.terminal_size;

        let padding_width_percent = 0.15;
        let padding_width_cols = (size.cols as f32 * padding_width_percent) as usize;
        let desired_width = (size.cols - padding_width_cols).min(size.cols);
        let avail_pixel_width =
            size.cols as f32 * term_window.render_metrics.cell_size.width as f32;
        let desired_pixel_width =
            desired_width as f32 * term_window.render_metrics.cell_size.width as f32;
        let x_adjust = (avail_pixel_width - desired_pixel_width) / 2.0;
        let panel_width = desired_pixel_width;

        let panel_margin_percent = 0.25;
        let panel_margin_pixels = font.metrics().cell_height.0 as f32 * panel_margin_percent;
        let panel_padding_percent = 0.25;
        let panel_padding_pixels = font.metrics().cell_height.0 as f32 * panel_padding_percent as f32;
        let panel_border_pixels = 2.0;
        let prompt_element_height = font.metrics().cell_height.0 as f32 + panel_margin_pixels + panel_padding_pixels + panel_border_pixels;

        let background_color = cloned_pane.pane.palette().background.to_linear();
        let prompt_element = self.create_prompt_element(term_window, panel_width, background_color);

        let padding_height_percent = 0.05;
        let padding_height_pixels = dimensions.pixel_height as f32 * padding_height_percent;
        let full_height = (dimensions.pixel_height as f32) - (padding_height_pixels * 2.0);
        let half_height = ((full_height - prompt_element_height - (panel_padding_pixels * 2.0)  - (panel_margin_percent * 2.0)) / 2.0).floor();

        let metrics = RenderMetrics::with_font_metrics(&font.metrics());
        let max_rows_on_screen = (half_height / (metrics.cell_size.height as f32 )) as usize;
        *self.max_rows_on_screen.borrow_mut() = max_rows_on_screen;
        let size = term_window.terminal_size;

        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height().unwrap()
        } else {
            0.
        };
        let (padding_left, padding_top) = term_window.padding_left_top();
        let border = term_window.get_os_border();
        let top_pixel_y = (top_bar_height + padding_top + border.top.get() as f32) + (padding_height_pixels / 2.0);

        let mut result_elements = vec![ ];
        let matcher = SkimMatcherV2::default();
        if(*self.results_dirty.borrow())
        {
            unsafe {
                let mut pattern_str = std::ffi::CString::new(selection).expect("CStr::from_bytes_with_nul failed");
                let pattern = fzf_parse_pattern(
                    fzf_case_types_CaseSmart,
                    false,
                    pattern_str.as_ptr() as *mut c_char,
                    true,
                );
                let mut ms = self.commands.borrow_mut();
                ms.clear();
                if (!selection.is_empty()) {
                    let pn = term_window.get_active_pane_or_overlay();
                    match pn {
                        Some(pn_value) => {
                            let pnDim = pn_value.get_dimensions();
                            let rows = pnDim.scrollback_rows as StableRowIndex;
                            let (_first_row, lines) = pn_value.get_lines(0..rows);
                            for (idx, line) in lines.iter().enumerate() {
                                let c_string = std::ffi::CString::new(line.as_str().as_ref()).expect("CString::new failed");
                                let ptr =c_string.as_ptr();
                                let score = fzf_get_score(ptr as *const i8, pattern, self.slab);
                                if(score > 0) {
                                    let command = EricRow {
                                        brief: Cow::Owned(line.as_str().to_string()),
                                        rowIndex: _first_row + idx as StableRowIndex,
                                        first_y: 0
                                    };
                                    ms.push(((score as i32).into(), command));
                                }
                                //match matcher.fuzzy_match(line.as_str().as_ref(), selection.clone())
                                //{
                                //    Some(score) => {
                                //        let command = EricRow {
                                //            brief: Cow::Owned(line.as_str().to_string()),
                                //            rowIndex: _first_row + idx as StableRowIndex,
                                //            first_y: 0
                                //        };
                                //        ms.push((score, command));
                                //    },
                                //    None => {}
                                //}
                            }
                        },
                        None => {}
                    }
                    ms.sort_by(|a, b| a.0.cmp(&b.0).reverse());
                    *self.results_dirty.borrow_mut() = false;
                }
            }
        }

        let mut top_row = self.top_row.borrow_mut();
        let mut commands = self.commands.borrow_mut();
        if(commands.iter().count() > 0)
        {
            *top_row = commands[*self.selected_row.borrow()].1.rowIndex;
        }

        for (display_idx, mut c) in commands.iter_mut().take(max_rows_on_screen).enumerate() {
            let mut command = &mut c.1;
            let solid_bg_color: InheritableColor = term_window
                .config
                .command_palette_bg_color
                .to_linear()
                .into();
            let solid_fg_color: InheritableColor = term_window
                .config
                .command_palette_fg_color
                .to_linear()
                .into();

            let selected_row = *self.selected_row.borrow();
            let (bg, text) = if display_idx == selected_row {
                (solid_fg_color.clone(), solid_bg_color.clone())
            } else {
                (LinearRgba::TRANSPARENT.into(), solid_fg_color.clone())
            };

            let (label_bg, label_text) = if display_idx == selected_row {
                (solid_fg_color.clone(), solid_bg_color.clone())
            } else {
                (solid_bg_color.clone(), solid_fg_color.clone())
            };

            let label = command.brief.to_string();

            let mut attr = CellAttributes::default();
            if(display_idx == selected_row)
            {
                attr.set_foreground(TrueColor(*term_window.config.command_palette_bg_color));
            }
            else {
                attr.set_foreground(TrueColor(*term_window.config.command_palette_fg_color));
            }
            let mut line = Line::from_text(&label, &attr, 0, None);

            unsafe {
                let mut pattern_str = std::ffi::CString::new(selection).expect("CStr::from_bytes_with_nul failed");
                let pattern = fzf_parse_pattern(
                    fzf_case_types_CaseSmart,
                    false,
                    pattern_str.as_ptr() as *mut c_char,
                    true,
                );
                let c_string = std::ffi::CString::new(line.as_str().as_ref()).expect("CString::new failed");
                let ptr = c_string.as_ptr();
                let pos = fzf_get_positions(ptr, pattern, self.slab);
                if(!pos.is_null())
                {
                    let s = core::slice::from_raw_parts((*pos).data, (*pos).size);
                    for p in s {
                        if let Some(c) = line.cells_mut_for_attr_changes_only().get_mut(*p as usize) {
                            c.attrs_mut().set_foreground(color::AnsiColor::Red);
                        }
                    }
                }

                //let matcher = SkimMatcherV2::default();
                //if let Some(pos) = matcher.fuzzy_indices(&label, selection.clone()) {
                //    if let Some(first_index) = pos.1.get(0) {
                //        command.first_y = *first_index;
                //        for p in pos.1 {
                //            if let Some(c) = line.cells_mut_for_attr_changes_only().get_mut(p) {
                //                c.attrs_mut().set_foreground(color::AnsiColor::Red);
                //            } else {
                //                //println!("Cell at position {} not found.", p);
                //            }
                //        }
                //    }
                //} else {
                //    //println!("No indices found.");
                //}
            }

            let row = vec![
                Element::with_line(&font, &line, &term_window.palette().clone()),
            ];

            result_elements.push(
                Element::new(&font, ElementContent::Children(row))
                    .colors(ElementColors {
                        border: BorderColor::default(),
                        bg,
                        text,
                    })
                    .padding(BoxDimension {
                        left: Dimension::Cells(0.25),
                        right: Dimension::Cells(0.25),
                        top: Dimension::Cells(0.),
                        bottom: Dimension::Cells(0.),
                    })
                    .min_width(Some(Dimension::Percent(1.)))
                    .display(DisplayType::Block),
            );
        }

        let results_element = self.create_panel_element(
            term_window,
            panel_width,
            max_rows_on_screen as f32 * metrics.cell_size.height as f32,
            background_color,
            BorderColor::new(
                    term_window.config.command_palette_fg_color.to_linear().into(),
                ),
            ElementContent::Children(result_elements));

        let preview_border_element = self.create_panel_element(
            term_window,
            panel_width,
            half_height,
            background_color,
            BorderColor::new(
                    term_window.config.command_palette_fg_color.to_linear().into(),
                ),
            ElementContent::Children(vec![]));

        let combined = vec![preview_border_element, results_element, prompt_element];
        let element = self.create_panel_element(
            term_window,
            panel_width,
            full_height,
            background_color,
            BorderColor::default(),
            ElementContent::Children(combined));

        let computed = term_window.compute_element(
            &LayoutContext {
                width: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_width as f32,
                    pixel_cell: metrics.cell_size.height as f32,
                },
                height: DimensionContext {
                    dpi: dimensions.dpi as f32,
                    pixel_max: dimensions.pixel_height as f32,
                    pixel_cell: metrics.cell_size.height as f32,
                },
                bounds: euclid::rect(
                    padding_left + x_adjust,
                    top_pixel_y,
                    desired_pixel_width,
                    size.rows as f32 * term_window.render_metrics.cell_size.height as f32,
                ),
                metrics: &metrics,
                gl_state: term_window.render_state.as_ref().unwrap(),
                zindex: 100,
            }, &element)?;

        let rt = vec!(computed);
        self.element.borrow_mut().replace(rt);

        let gl_state = term_window.render_state.as_ref().unwrap();
        let layer = gl_state
            .layer_for_zindex(101)?;
        let mut layers = layer.quad_allocator();

        cloned_pane.left = cloned_pane.left;

        let inner_panel_padding = (panel_margin_pixels + panel_padding_pixels + panel_border_pixels + padding_left) * 2.0;
        term_window.paint_pane2(
            &cloned_pane,
            &mut layers,
            x_adjust + inner_panel_padding,
            top_pixel_y + inner_panel_padding,
            desired_pixel_width - (inner_panel_padding),
            half_height, *top_row)?;

        Ok(Ref::map(self.element.borrow(), |v| {
            v.as_ref().unwrap().as_slice()
        }))
    }

    fn reconfigure(&self, term_window: &mut TermWindow) {
        self.element.borrow_mut().take();
    }
}
