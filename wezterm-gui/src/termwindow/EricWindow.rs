include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

use std::borrow::Cow;
use std::cell::{Ref, RefCell};
use std::sync::{Arc, mpsc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use parking_lot::RwLock;

use config::{Dimension, SrgbaTuple};
use mux::pane::{LogicalLine, Pane};
use mux::pane::Pattern::CaseInSensitiveString;
use termwiz::cell::CellAttributes;
use termwiz::color;
use termwiz::color::ColorSpec::TrueColor;
use termwiz::surface::Line;
use wezterm_term::{KeyCode, KeyModifiers, MouseEvent, StableRowIndex};
use window::color::LinearRgba;
use window::{Modifiers, WindowOps};

use crate::termwindow::{DimensionContext, TermWindow};
use crate::termwindow::box_model::*;
use crate::termwindow::modal::Modal;
use crate::termwindow::render::corners::{
    BOTTOM_LEFT_ROUNDED_CORNER, BOTTOM_RIGHT_ROUNDED_CORNER, TOP_LEFT_ROUNDED_CORNER,
    TOP_RIGHT_ROUNDED_CORNER,
};
use crate::utilsprites::RenderMetrics;

pub struct EricRow {
    pub row_index: StableRowIndex,
    pub first_y: usize,
    pub positions: Vec<u32>
}
pub struct EricWindow {
    element: RefCell<Option<Vec<ComputedElement>>>,
    selection: RefCell<String>,
    selected_row: RefCell<usize>,
    top_row: RefCell<StableRowIndex>,
    max_rows_on_screen: RefCell<usize>,
    ms: RwLock<Vec<(i32, EricRow)>>,
    row_indexes: RefCell<Vec<EricRow>>,
    fuzzy_searcher: Arc<FuzzySearcher>
}

impl EricWindow{
    pub fn new(term_window: &mut TermWindow) -> Self {
        unsafe {
            let pane = term_window.get_active_pane_or_overlay().unwrap();
            let pn_dim = pane.get_dimensions();
            let rows = pn_dim.scrollback_rows as StableRowIndex;

            let logical_lines = pane.get_logical_lines(0..rows);
            let (_first_row, lines) = pane.get_lines(0..rows);
            Self {
                element: RefCell::new(None),
                selection: RefCell::new(String::new()),
                row_indexes: RefCell::new(Vec::new()),
                ms: RwLock::new(Vec::new()),
                selected_row: RefCell::new(0),
                top_row: RefCell::new(0),
                max_rows_on_screen: RefCell::new(0),
                fuzzy_searcher: FuzzySearcher::new(logical_lines),
            }
        }
    }

    fn start_fuzzy_search(&self, term_window: &mut TermWindow) {
        let selection = self.selection.borrow().clone();
        match term_window.get_active_pane_or_overlay(){
            Some(pn_value) => {
                let fuzzy_searcher_clone = Arc::clone(&self.fuzzy_searcher);
                fuzzy_searcher_clone.search(selection.as_ref(), pn_value, term_window);
                term_window.invalidate_modal();
            },
            None => {}
        };
    }

    fn updated_input(&self) {
        *self.selected_row.borrow_mut() = 0;
        *self.top_row.borrow_mut() = 0;
    }

    fn move_up(&self) {
        let mut row = self.selected_row.borrow_mut();
        *row = row.saturating_sub(1);
        let mut top_row = self.top_row.borrow_mut();
        let commands = self.fuzzy_searcher.results.read().unwrap();
        *top_row = commands[*row].row_index;
    }

    fn move_down(&self) {
        let mut row = self.selected_row.borrow_mut();
        let commands = self.fuzzy_searcher.results.read().unwrap();
        if(*row + 1 < commands.iter().count())
        {
            *row = row.saturating_add(1);
            let mut top_row = self.top_row.borrow_mut();
            if(*row < commands.iter().count())
            {
                *top_row = commands[*row].row_index;
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
            0.0,
            0.0,
            0.0
        )
    }

    fn create_panel_element(
        &self,
        term_window: &TermWindow,
        panel_width: f32,
        panel_height: f32,
        background_color: LinearRgba,
        border_color: BorderColor,
        content: ElementContent,
        margin_cell_percent: f32,
        padding_cell_percent: f32,
        border_pixels: f32
    ) -> Element {
        let font = term_window
            .fonts
            .default_font()
            .expect("to resolve font");

        Element::new(&font, content)
            .colors(ElementColors {
                border: BorderColor::default(),
                bg: background_color.into(),
                text: term_window.config.command_palette_fg_color.to_linear().into(),
            })
            .colors(ElementColors {
                border: border_color,
                bg: background_color.into(),
                text: term_window.config.command_palette_fg_color.to_linear().into(),
            })
            .margin(BoxDimension {
                left: Dimension::Cells(margin_cell_percent),
                right: Dimension::Cells(margin_cell_percent),
                top: Dimension::Cells(margin_cell_percent),
                bottom: Dimension::Cells(margin_cell_percent),
            })
            .padding(BoxDimension {
                left: Dimension::Cells(padding_cell_percent),
                right: Dimension::Cells(padding_cell_percent),
                top: Dimension::Cells(padding_cell_percent),
                bottom: Dimension::Cells(padding_cell_percent),
            })
            .border(BoxDimension::new(Dimension::Pixels(border_pixels)))
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

                //let commands = self.commands.borrow();
                let y = self.fuzzy_searcher.results.read().unwrap()[*row].row_index;;
                let x = self.fuzzy_searcher.results.read().unwrap()[*row].first_y;

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
                {
                    let mut selection = self.selection.borrow_mut();
                    selection.pop();
                }
                self.updated_input();
                self.start_fuzzy_search(term_window);
            }
            (KeyCode::Char(c), KeyModifiers::NONE) | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                {
                    let mut selection = self.selection.borrow_mut();
                    selection.push(c);
                }
                self.updated_input();
                self.start_fuzzy_search(term_window);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn computed_element(&self, term_window: &mut TermWindow) -> anyhow::Result<Ref<[ComputedElement]>> {
        let panes = term_window.get_panes_to_render();
        let mut cloned_pane = panes[0].clone();

        let font = term_window
            .fonts
            .default_font()
            .expect("to resolve font");

        let dimensions = term_window.dimensions;
        let size = term_window.terminal_size;

        let avail_pixel_width =
            size.cols as f32 * term_window.render_metrics.cell_size.width as f32;


        let proposed_window_to_modal_padding_percent = 0.15;
        let proposed_window_to_modal_padding_pixels = dimensions.pixel_width as f32 * proposed_window_to_modal_padding_percent;

        let (padding_left, padding_top) = term_window.padding_left_top();
        let padding_width_percent = 0.15;
        let padding_width_cols = (size.cols as f32 * padding_width_percent) as usize;
        let desired_width = (size.cols - padding_width_cols).min(size.cols);
        let desired_pixel_width =
            desired_width as f32 * term_window.render_metrics.cell_size.width as f32;

        let panel_margin_percent = 0.50;
        let panel_margin_pixels = term_window.render_metrics.cell_size.width as f32 * panel_margin_percent;
        let panel_padding_percent = 0.50;
        let panel_padding_pixels = term_window.render_metrics.cell_size.width as f32 * panel_padding_percent;
        let panel_border_pixels = 2.0;
        let prompt_element_height = font.metrics().cell_height.0 as f32 + panel_margin_pixels + panel_border_pixels;
        let panel_decoration_pixels = (panel_margin_pixels + panel_padding_pixels);

        let proposed_content_width_pixels = dimensions.pixel_width as f32 - proposed_window_to_modal_padding_pixels - panel_decoration_pixels;
        let proposed_full_height = dimensions.pixel_height as f32 - proposed_window_to_modal_padding_pixels - panel_decoration_pixels;
        let proposed_half_height = ((proposed_full_height - prompt_element_height - panel_decoration_pixels) / 2.0).floor();
        let content_width_cells = (proposed_content_width_pixels / term_window.render_metrics.cell_size.width as f32).floor();
        let content_width_pixels = content_width_cells * term_window.render_metrics.cell_size.width as f32;
        let content_height_cells = (proposed_half_height / term_window.render_metrics.cell_size.height as f32).floor();
        let content_height_pixels = content_height_cells * term_window.render_metrics.cell_size.height as f32;

        let real_panel_width = content_width_pixels + (panel_decoration_pixels * 2.0) + panel_border_pixels + panel_border_pixels;
        let real_panel_height = content_height_pixels + panel_decoration_pixels;
        let real_modal_to_window_width_padding = (dimensions.pixel_width as f32 - real_panel_width) / 2.0;

        let x_adjust = real_modal_to_window_width_padding;
        let x_adjust_content = x_adjust + (panel_decoration_pixels  * 2.0) + panel_border_pixels;
        let background_color = cloned_pane.pane.palette().background.to_linear();

        let selection = self.selection.borrow();
        let selection = selection.as_str();
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
        let prompt_element = self.create_panel_element(
            term_window,
            real_panel_width,
            1.0,
            background_color,
            BorderColor::new(
                term_window.config.command_palette_fg_color.to_linear().into(),
            ),
            ElementContent::Children(prompt_elements),
            panel_margin_percent,
            panel_padding_percent,
            2.0
        );

        let top_bar_height = if term_window.show_tab_bar && !term_window.config.tab_bar_at_bottom {
            term_window.tab_bar_pixel_height().unwrap()
        } else {
            0.
        };

        let padding_height_percent = 0.10;
        let padding_height_pixels = (dimensions.pixel_height as f32 - top_bar_height) * padding_height_percent;
        let full_height = (dimensions.pixel_height as f32) - (padding_height_pixels * 2.0) - top_bar_height;
        let half_height = ((full_height - prompt_element_height - (panel_padding_pixels * 2.0)  - (panel_margin_percent * 2.0)) / 2.0).floor();

        let metrics = RenderMetrics::with_font_metrics(&font.metrics());
        let max_rows_on_screen = (half_height / (metrics.cell_size.height as f32 )) as usize;
        *self.max_rows_on_screen.borrow_mut() = max_rows_on_screen;
        let size = term_window.terminal_size;

        let border = term_window.get_os_border();
        let top_pixel_y = padding_top + top_bar_height + real_modal_to_window_width_padding;
        let top_pixel_y_content = top_pixel_y + (panel_decoration_pixels * 2.0) + panel_border_pixels;

        let mut result_elements = vec![ ];

        let mut top_row = self.top_row.borrow_mut();
        let a = self.fuzzy_searcher.results.read().unwrap();
        if(a.iter().count() > 0)
        {
            *top_row = a[*self.selected_row.borrow()].row_index;
        }

        for (display_idx, mut c) in a.iter().take(max_rows_on_screen).enumerate() {
            let mut command = &mut c;
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

            let mut attr = CellAttributes::default();
            if(display_idx == selected_row)
            {
                attr.set_foreground(TrueColor(*term_window.config.command_palette_bg_color));
            }
            else {
                attr.set_foreground(TrueColor(*term_window.config.command_palette_fg_color));
            }

            let logical_rows = &cloned_pane.pane.get_logical_lines(command.row_index..command.row_index + 1);
            if let Some(logical_row) = logical_rows.first() {
                for line in &logical_row.physical_lines {

                    let label_str = line.as_str();
                    let mut line = Line::from_text(&label_str, &attr, 0, None);

                    for p in c.positions.iter() {
                        if let Some(c) = line.cells_mut_for_attr_changes_only().get_mut(*p as usize) {
                            c.attrs_mut().set_foreground(color::AnsiColor::Red);
                        }
                    }

                    let row = vec![
                        Element::with_line(&font, &line, &term_window.palette().clone()),
                    ];

                    result_elements.push(
                        Element::new(&font, ElementContent::Children(row))
                            .colors(ElementColors {
                                border: BorderColor::default(),
                                bg: bg.clone(),
                                text: text.clone(),
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
            } else {
            }
        }

        let results_element = self.create_panel_element(
            term_window,
            real_panel_width,
            max_rows_on_screen as f32 * metrics.cell_size.height as f32,
            background_color,
            BorderColor::new(
                    term_window.config.command_palette_fg_color.to_linear().into(),
                ),
            ElementContent::Children(result_elements),
            panel_margin_percent,
            panel_padding_percent,
            2.0
        );

        let preview_border_element = self.create_panel_element(
            term_window,
            real_panel_width,
            half_height,
            background_color,
            BorderColor::new(
                    term_window.config.command_palette_fg_color.to_linear().into(),
                ),
            ElementContent::Children(vec![]),
            panel_margin_percent,
            panel_padding_percent,
            2.0
        );

        let combined = vec![preview_border_element, results_element, prompt_element];
        let element = self.create_panel_element(
            term_window,
            real_panel_width,
            full_height,
            background_color,
            BorderColor::default(),
            ElementContent::Children(combined),
        0.0,
        0.0,
        0.0);

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
                    x_adjust,
                    top_pixel_y,
                    real_panel_width,
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

        let inner_panel_padding = (panel_margin_pixels + panel_padding_pixels + panel_border_pixels) * 2.0;
        term_window.paint_pane2(
            &cloned_pane,
            &mut layers,
            x_adjust_content,
            top_pixel_y_content,
            content_width_pixels,
            half_height,
            *top_row)?;

        Ok(Ref::map(self.element.borrow(), |v| {
            v.as_ref().unwrap().as_slice()
        }))
    }

    fn reconfigure(&self, term_window: &mut TermWindow) {
        self.element.borrow_mut().take();
    }
}

struct SearchTask {
    selection: String,
    pane: Arc<dyn Pane>,
    //term_window: Arc<TermWindow>, // Change Rc to Arc
}

pub struct FuzzySearcher {
    results: Arc<std::sync::RwLock<Vec<EricRow>>>,
    cancel_flag: Arc<AtomicBool>,
    task_sender: Arc<Mutex<Sender<SearchTask>>>,
    lines: Vec<LogicalLine>,
    task_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl FuzzySearcher {
    pub fn new(lines: Vec<LogicalLine>) -> Arc<Self> {
        let (task_sender, task_receiver) = mpsc::channel();

        let mut searcher = Arc::new(FuzzySearcher {
            results: Arc::new(std::sync::RwLock::new(Vec::new())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            task_sender: Arc::new(Mutex::new(task_sender)),
            lines,
            task_thread: Arc::new(Mutex::new(None))
        });

        searcher
    }
    pub fn stop(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        if let Some(thread) = self.task_thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    fn worker_thread(self: Arc<Self>, task_receiver: Receiver<SearchTask>) {
        for task in task_receiver {
            let self_clone = Arc::clone(&self);
            self_clone.cancel_flag.store(false, Ordering::SeqCst);
            self_clone.perform_search(task.selection, task.pane);
        }
    }

    fn perform_search(self: Arc<Self>, selection: String, pane: Arc<dyn Pane>) {
        let cancel_flag_clone = Arc::clone(&self.cancel_flag);

        unsafe {
            let pn_dim = pane.get_dimensions();
            let rows = pn_dim.scrollback_rows as StableRowIndex;
            let _first_row = 0;
            if !selection.is_empty() {
                let pattern_str = std::ffi::CString::new(selection).expect("CString::new failed");
                let slab = fzf_make_default_slab();
                let pattern = fzf_parse_pattern(
                    0, // Replace with actual value
                    false,
                    pattern_str.as_ptr() as *mut i8,
                    true,
                );

                let mut temp = vec![];
                for (idx, line) in self.lines.iter().enumerate() {
                    if cancel_flag_clone.load(Ordering::SeqCst) {
                        return;
                    }
                    let c_string = std::ffi::CString::new(line.logical.as_str().as_ref()).expect("CString::new failed");
                    let ptr = c_string.as_ptr();
                    let score = fzf_get_score(ptr, pattern, slab);

                    if score > 0 {
                        temp.push((score, _first_row + idx as StableRowIndex, c_string));
                    }
                }

                let mut ms = vec![];
                temp.sort_by(|a, b| a.0.cmp(&b.0).reverse());
                for (display_idx, mut c) in temp.iter_mut().take(100).enumerate() {
                    //let line = c.2;
                    //let c_string = std::ffi::CString::new(line.as_str().as_ref()).expect("CString::new failed");
                    //let ptr = c_string.as_ptr();
                    let pos = fzf_get_positions(c.2.as_ptr(), pattern, slab);
                    if !pos.is_null() {
                        let s = core::slice::from_raw_parts((*pos).data, (*pos).size);
                        let mut posVec = vec![];
                        for &p in s.iter() {
                            posVec.push(p);
                        }
                        fzf_free_positions(pos);

                        let first_y: usize = *posVec.last().unwrap_or(&0) as usize;
                        let command = EricRow {
                            //brief: Cow::Owned(c.2),
                            row_index: c.1 as StableRowIndex,
                            first_y: first_y,
                            positions: posVec,
                        };
                        ms.push(command);
                    }
                }

                fzf_free_pattern(pattern);
                fzf_free_slab(slab);

                if cancel_flag_clone.load(Ordering::SeqCst) {
                    return;
                }

                let mut results = self.results.write().unwrap();
                *results = ms;
            }
        }
    }

    pub fn search(self: Arc<Self>, selection: &str, pane: Arc<dyn Pane>, term_window: &TermWindow) {
        self.cancel_flag.store(true, Ordering::SeqCst);

        let task = SearchTask {
            selection: selection.to_string(),
            pane,
        };

        if selection.is_empty() {
            self.results.write().unwrap().clear();
        } else {
            let self_clone = Arc::clone(&self);
            thread::spawn(move || {
                self_clone.cancel_flag.store(false, Ordering::SeqCst);
                self_clone.perform_search(task.selection, task.pane);
            });
        }
    }
}

impl Clone for FuzzySearcher {
    fn clone(&self) -> Self {
        FuzzySearcher {
            results: Arc::clone(&self.results),
            cancel_flag: Arc::clone(&self.cancel_flag),
            task_sender: Arc::clone(&self.task_sender),
            lines: self.lines.clone(),
            task_thread: Arc::new(Mutex::new(None))
        }
    }
}