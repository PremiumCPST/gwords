//! Table widget embedded in the main editor through a `gtk::TextChildAnchor`.
//! Each cell is an `Editor::new_embedded()`.

use gtk::glib;
use gtk::glib::subclass::Signal;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use gtk::gio;
use crate::editor::convert;

use crate::document::{Paragraph, Table};
use crate::editor::Editor;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TableWidget {
        pub grid: gtk::Grid,
        /// Cell editors, row-major.
        pub cells: RefCell<Vec<Vec<Editor>>>,
        /// Cell targeted by the last context menu.
        pub ctx: Cell<(usize, usize)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TableWidget {
        const NAME: &'static str = "GwTableWidget";
        type Type = super::TableWidget;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for TableWidget {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_orientation(gtk::Orientation::Vertical);
            obj.add_css_class("gw-table");
            obj.set_halign(gtk::Align::Start);
            self.grid.add_css_class("gw-table-grid");
            self.grid.set_column_homogeneous(true);
            self.grid.set_row_spacing(0);
            self.grid.set_column_spacing(0);
            self.grid.set_hexpand(true);
            obj.append(&self.grid);
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    // Content or structure changed (mark the document modified).
                    Signal::builder("changed").build(),
                    // The format state of a cell editor may have changed.
                    Signal::builder("cell-format-changed").build(),
                ]
            })
        }
    }
    impl WidgetImpl for TableWidget {}
    impl BoxImpl for TableWidget {}
}

glib::wrapper! {
    pub struct TableWidget(ObjectSubclass<imp::TableWidget>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl TableWidget {
    /// Builds the grid from `table` (made rectangular, at least 1x1).
    pub fn new(table: &Table) -> Self {
        let obj: Self = glib::Object::new();
        let mut table = table.clone();
        table.make_rectangular();
        let rows: Vec<Vec<Editor>> = table
            .rows
            .iter()
            .map(|r| r.cells.iter().map(|c| obj.make_cell(&c.paragraphs)).collect())
            .collect();
        *obj.imp().cells.borrow_mut() = rows;
        obj.relayout();
        obj.install_actions();
        obj
    }

    pub fn to_table(&self) -> Table {
        let cells = self.imp().cells.borrow();
        Table {
            rows: cells
                .iter()
                .map(|row| crate::document::TableRow {
                    cells: row
                        .iter()
                        .map(|ed| crate::document::TableCell {
                            paragraphs: convert::buffer_to_paragraphs(&ed.buffer()),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn row_count(&self) -> usize {
        self.imp().cells.borrow().len()
    }

    pub fn column_count(&self) -> usize {
        self.imp().cells.borrow().first().map_or(0, Vec::len)
    }

    /// Cell editors, row-major.
    pub fn cell_editors(&self) -> Vec<Editor> {
        self.imp().cells.borrow().iter().flatten().cloned().collect()
    }

    pub fn cell_editor(&self, row: usize, col: usize) -> Option<Editor> {
        self.imp().cells.borrow().get(row).and_then(|r| r.get(col)).cloned()
    }

    /// (row, col) of the cell containing the focus.
    pub fn focused_cell(&self) -> Option<(usize, usize)> {
        let focus = self.root().and_then(|r| r.focus())?;
        let cells = self.imp().cells.borrow();
        for (r, row) in cells.iter().enumerate() {
            for (c, ed) in row.iter().enumerate() {
                let w: &gtk::Widget = ed.upcast_ref();
                if &focus == w || focus.is_ancestor(w) {
                    return Some((r, c));
                }
            }
        }
        None
    }

    /// Structure edits; indices are clamped. Each emits "changed".
    pub fn insert_row(&self, at: usize) {
        let cols = self.column_count().max(1);
        let at = at.min(self.row_count());
        let row: Vec<Editor> = (0..cols).map(|_| self.make_cell(&[])).collect();
        for ed in &row {
            ed.set_opacity(0.0);
        }
        self.imp().cells.borrow_mut().insert(at, row.clone());
        self.relayout();
        for ed in &row {
            Self::fade_in(ed);
        }
        self.emit_changed();
    }

    pub fn insert_column(&self, at: usize) {
        if self.row_count() == 0 {
            self.insert_row(0);
            return;
        }
        let at = at.min(self.column_count());
        let mut added = Vec::new();
        {
            let mut cells = self.imp().cells.borrow().clone();
            for row in cells.iter_mut() {
                let ed = self.make_cell(&[]);
                ed.set_opacity(0.0);
                row.insert(at, ed.clone());
                added.push(ed);
            }
            *self.imp().cells.borrow_mut() = cells;
        }
        self.relayout();
        for ed in &added {
            Self::fade_in(ed);
        }
        self.emit_changed();
    }

    /// Removes a row. Removing the last row leaves a table with 0 rows; the
    /// main editor watches "changed" and deletes the anchor when
    /// `row_count() == 0` (this is also how "Supprimer le tableau" works).
    pub fn remove_row(&self, at: usize) {
        {
            let mut cells = self.imp().cells.borrow_mut();
            if cells.is_empty() {
                return;
            }
            let at = at.min(cells.len() - 1);
            cells.remove(at);
        }
        self.relayout();
        self.emit_changed();
    }

    /// Removes a column. Removing the last column leaves 0 rows and 0 columns.
    pub fn remove_column(&self, at: usize) {
        let cols = self.column_count();
        if cols == 0 {
            return;
        }
        let at = at.min(cols - 1);
        {
            let mut cells = self.imp().cells.borrow_mut();
            if cols == 1 {
                cells.clear();
            } else {
                for row in cells.iter_mut() {
                    row.remove(at);
                }
            }
        }
        self.relayout();
        self.emit_changed();
    }

    /// Width the table should occupy (the page's text width); called by the
    /// main editor when its width changes.
    pub fn set_available_width(&self, px: i32) {
        self.set_size_request(px.max(1), -1);
    }

    pub fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("changed", false, move |args| {
            f(&args[0].get::<Self>().unwrap());
            None
        })
    }

    pub fn connect_cell_format_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("cell-format-changed", false, move |args| {
            f(&args[0].get::<Self>().unwrap());
            None
        })
    }

    // ---- internals -------------------------------------------------------

    fn emit_changed(&self) {
        self.emit_by_name::<()>("changed", &[]);
    }

    fn fade_in(ed: &Editor) {
        let target = adw::CallbackAnimationTarget::new(glib::clone!(
            #[weak]
            ed,
            move |v| ed.set_opacity(v)
        ));
        let anim = adw::TimedAnimation::new(ed, 0.0, 1.0, 220, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        adw::prelude::AnimationExt::play(&anim);
    }

    fn position_of(&self, ed: &Editor) -> Option<(usize, usize)> {
        let cells = self.imp().cells.borrow();
        for (r, row) in cells.iter().enumerate() {
            if let Some(c) = row.iter().position(|e| e == ed) {
                return Some((r, c));
            }
        }
        None
    }

    /// Re-attaches every cell to the grid at its (row, col) and refreshes
    /// the border style classes. Keeps the focused cell focused.
    fn relayout(&self) {
        let imp = self.imp();
        let focused = self.focused_cell().and_then(|(r, c)| self.cell_editor(r, c));
        while let Some(child) = imp.grid.first_child() {
            imp.grid.remove(&child);
        }
        for (r, row) in imp.cells.borrow().iter().enumerate() {
            for (c, ed) in row.iter().enumerate() {
                imp.grid.attach(ed, c as i32, r as i32, 1, 1);
                if r == 0 { ed.add_css_class("first-row") } else { ed.remove_css_class("first-row") }
                if c == 0 { ed.add_css_class("first-col") } else { ed.remove_css_class("first-col") }
            }
        }
        if let Some(ed) = focused {
            if ed.parent().is_some() {
                ed.grab_focus();
            }
        }
    }

    fn focus_cell(&self, r: usize, c: usize, end: bool) {
        if let Some(ed) = self.cell_editor(r, c) {
            ed.grab_focus();
            let buf = ed.buffer();
            let it = if end { buf.end_iter() } else { buf.start_iter() };
            buf.place_cursor(&it);
        }
    }

    fn make_cell(&self, paragraphs: &[Paragraph]) -> Editor {
        let ed = Editor::new_embedded();
        ed.set_hexpand(true);
        ed.set_wrap_mode(gtk::WrapMode::WordChar);
        if !paragraphs.is_empty() {
            convert::load_paragraphs(&ed.buffer(), paragraphs);
        }

        let buf = ed.buffer();
        buf.connect_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.emit_changed()
        ));
        buf.connect_apply_tag(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _, _, _| this.emit_changed()
        ));
        buf.connect_remove_tag(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_, _, _, _| this.emit_changed()
        ));
        ed.connect_format_changed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            move |_| this.emit_by_name::<()>("cell-format-changed", &[])
        ));

        // Keyboard navigation, before the editor's own handling.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            ed,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| this.on_key(&ed, key, state)
        ));
        ed.add_controller(keys);

        // Context menu.
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.connect_pressed(glib::clone!(
            #[weak(rename_to = this)]
            self,
            #[weak]
            ed,
            move |g, _, x, y| {
                g.set_state(gtk::EventSequenceState::Claimed);
                this.show_menu(&ed, x, y);
            }
        ));
        ed.add_controller(click);
        ed
    }

    fn on_key(&self, ed: &Editor, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
        use gtk::gdk::Key;
        let Some((r, c)) = self.position_of(ed) else {
            return glib::Propagation::Proceed;
        };
        let (rows, cols) = (self.row_count(), self.column_count());
        let plain = !state.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        match key {
            Key::Tab if !state.contains(gtk::gdk::ModifierType::SHIFT_MASK) => {
                if c + 1 < cols {
                    self.focus_cell(r, c + 1, false);
                } else if r + 1 < rows {
                    self.focus_cell(r + 1, 0, false);
                } else {
                    self.insert_row(rows);
                    self.focus_cell(rows, 0, false);
                }
                glib::Propagation::Stop
            }
            Key::Tab | Key::ISO_Left_Tab => {
                if c > 0 {
                    self.focus_cell(r, c - 1, true);
                } else if r > 0 {
                    self.focus_cell(r - 1, cols - 1, true);
                }
                glib::Propagation::Stop
            }
            Key::Up if plain && r > 0 => {
                let buf = ed.buffer();
                if buf.iter_at_mark(&buf.get_insert()).line() == 0 {
                    self.focus_cell(r - 1, c, true);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            Key::Down if plain && r + 1 < rows => {
                let buf = ed.buffer();
                if buf.iter_at_mark(&buf.get_insert()).line() >= buf.line_count() - 1 {
                    self.focus_cell(r + 1, c, false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn show_menu(&self, ed: &Editor, x: f64, y: f64) {
        let Some(pos) = self.position_of(ed) else { return };
        self.imp().ctx.set(pos);
        ed.grab_focus();
        let menu = gio::Menu::new();
        let ins = gio::Menu::new();
        ins.append(Some("Insérer une ligne au-dessus"), Some("table.row-above"));
        ins.append(Some("Insérer une ligne en dessous"), Some("table.row-below"));
        ins.append(Some("Insérer une colonne à gauche"), Some("table.col-left"));
        ins.append(Some("Insérer une colonne à droite"), Some("table.col-right"));
        let del = gio::Menu::new();
        del.append(Some("Supprimer la ligne"), Some("table.delete-row"));
        del.append(Some("Supprimer la colonne"), Some("table.delete-column"));
        del.append(Some("Supprimer le tableau"), Some("table.delete-table"));
        menu.append_section(None, &ins);
        menu.append_section(None, &del);
        let pop = gtk::PopoverMenu::from_model(Some(&menu));
        pop.set_parent(ed);
        pop.set_has_arrow(false);
        pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        pop.popup();
    }

    fn install_actions(&self) {
        let group = gio::SimpleActionGroup::new();
        let add = |name: &str, f: fn(&TableWidget, usize, usize)| {
            let a = gio::SimpleAction::new(name, None);
            a.connect_activate(glib::clone!(
                #[weak(rename_to = this)]
                self,
                move |_, _| {
                    let (r, c) = this.imp().ctx.get();
                    f(&this, r, c);
                }
            ));
            group.add_action(&a);
        };
        add("row-above", |t, r, _| t.insert_row(r));
        add("row-below", |t, r, _| t.insert_row(r + 1));
        add("col-left", |t, _, c| t.insert_column(c));
        add("col-right", |t, _, c| t.insert_column(c + 1));
        add("delete-row", |t, r, _| t.remove_row(r));
        add("delete-column", |t, _, c| t.remove_column(c));
        // The table cannot remove its own anchor: it empties itself and the
        // main editor deletes the anchor once `row_count() == 0`.
        add("delete-table", |t, _, _| {
            while t.row_count() > 0 {
                t.remove_row(0);
            }
        });
        self.insert_action_group("table", Some(&group));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::document::{Run, TableCell, TableRow};

    fn cell(text: &str) -> TableCell {
        TableCell {
            paragraphs: vec![Paragraph { runs: vec![Run { text: text.into(), ..Default::default() }], ..Default::default() }],
        }
    }

    /// Needs GTK initialised on the current thread (not a `#[test]`).
    pub(crate) fn gtk_table() {
        let table = Table {
            rows: (0..2)
                .map(|r| TableRow { cells: (0..3).map(|c| cell(&format!("r{r}c{c}"))).collect() })
                .collect(),
        };
        let w = TableWidget::new(&table);
        assert_eq!(w.row_count(), 2);
        assert_eq!(w.column_count(), 3);
        assert_eq!(w.cell_editors().len(), 6);
        let back = w.to_table();
        assert_eq!(back.rows.len(), 2);
        for (r, row) in back.rows.iter().enumerate() {
            for (c, cl) in row.cells.iter().enumerate() {
                assert_eq!(cl.plain_text(), format!("r{r}c{c}"));
            }
        }
        let changed = std::rc::Rc::new(Cell::new(0));
        let ch = changed.clone();
        w.connect_changed(move |_| ch.set(ch.get() + 1));
        w.insert_row(1);
        assert_eq!((w.row_count(), w.column_count()), (3, 3));
        assert_eq!(w.to_table().rows[1].cells[0].plain_text(), "");
        w.insert_column(0);
        assert_eq!((w.row_count(), w.column_count()), (3, 4));
        w.remove_column(0);
        w.remove_row(1);
        assert_eq!((w.row_count(), w.column_count()), (2, 3));
        assert_eq!(w.to_table(), back);
        assert!(changed.get() >= 4);
        w.remove_row(0);
        w.remove_row(0);
        assert_eq!(w.row_count(), 0);
    }
}
