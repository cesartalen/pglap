mod db;
mod history;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, List, ListState, Paragraph, Wrap};
use ratatui::Frame;
use ratatui_textarea::TextArea;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{error::Error, fs, thread, time::Duration};

const QUERIES: &str = "queries";
const HELP: &str = " tab: focus   enter: load query   ctrl+r: run   ctrl+s: save   ctrl+q: quit";

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Databases,
    Queries,
    Name,
    Runs,
    Editor,
}

struct App {
    focus: Focus,
    databases: Vec<String>,
    database: ListState,
    queries: Vec<String>,
    query: ListState,
    name: String,
    runs: String,
    editor: TextArea<'static>,
    result: String,
    sender: Sender<String>,
    receiver: Receiver<String>,
}

impl App {
    fn new() -> Result<Self, postgres::Error> {
        let (sender, receiver) = channel();
        Ok(Self {
            focus: Focus::Databases,
            databases: db::list_databases()?,
            database: ListState::default().with_selected(Some(0)),
            queries: saved_queries(),
            query: ListState::default().with_selected(Some(0)),
            name: String::new(),
            runs: "10".into(),
            editor: editor(""),
            result: "Write a query, then press ctrl+r".into(),
            sender,
            receiver,
        })
    }

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (self.focus, key.code) {
            (_, KeyCode::Char('r')) if ctrl => self.run(),
            (_, KeyCode::Char('s')) if ctrl => self.save(),
            (_, KeyCode::Tab) => self.focus = self.focus.next(),
            (Focus::Databases, code) => step(&mut self.database, code),
            (Focus::Queries, KeyCode::Enter) => self.load(),
            (Focus::Queries, code) => step(&mut self.query, code),
            (Focus::Name, code) => edit(&mut self.name, code),
            (Focus::Runs, KeyCode::Char(c)) if !c.is_ascii_digit() => {}
            (Focus::Runs, code) => edit(&mut self.runs, code),
            (Focus::Editor, _) => drop(self.editor.input(key)),
        }
    }

    fn selected_database(&self) -> String {
        let index = self.database.selected().unwrap_or(0).min(self.databases.len() - 1);
        self.databases[index].clone()
    }

    fn load(&mut self) {
        let Some(name) = self.query.selected().and_then(|i| self.queries.get(i)) else { return };
        let sql = fs::read_to_string(format!("{QUERIES}/{name}.sql")).unwrap_or_default();
        self.name = name.clone();
        self.editor = editor(&sql);
        self.focus = Focus::Editor;
    }

    fn save(&mut self) {
        if self.name.is_empty() {
            self.result = "Give the query a name first".into();
            return;
        }
        let saved = fs::create_dir_all(QUERIES)
            .and_then(|_| fs::write(format!("{QUERIES}/{}.sql", self.name), self.editor.lines().join("\n")));
        self.result = match saved {
            Ok(()) => format!("Saved {}", self.name),
            Err(error) => error.to_string(),
        };
        self.queries = saved_queries();
    }

    fn run(&mut self) {
        let database = self.selected_database();
        let sql = self.editor.lines().join("\n");
        let runs = self.runs.parse().unwrap_or(1).max(1);
        let name = self.name.clone();
        let sender = self.sender.clone();
        thread::spawn(move || {
            let result = match db::benchmark(&database, &sql, runs, &sender) {
                Ok(times) => {
                    let saved = history::save(&db::host(), &database, &name, &sql, &times);
                    let note = saved.err().map(|error| format!("\nhistory not saved: {error}")).unwrap_or_default();
                    db::summary(times) + &note
                }
                Err(error) => db::message(error),
            };
            let _ = sender.send(result);
        });
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [main, help] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        let [left, right] = Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).areas(main);
        let [databases, queries] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(left);
        let [inputs, sql, result] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0), Constraint::Length(6)]).areas(right);
        let [name, runs] = Layout::horizontal([Constraint::Min(0), Constraint::Length(12)]).areas(inputs);

        let focus = self.focus;
        let block = |title: &str, pane| {
            let color = if focus == pane { Color::Yellow } else { Color::DarkGray };
            Block::bordered().title(title.to_string()).border_style(color)
        };
        let list = |frame: &mut Frame, items: &[String], state: &mut ListState, area: Rect, block| {
            let list = List::new(items.iter().map(String::as_str))
                .block(block)
                .highlight_style(Modifier::REVERSED);
            frame.render_stateful_widget(list, area, state);
        };

        list(frame, &self.databases, &mut self.database, databases, block("Databases", Focus::Databases));
        list(frame, &self.queries, &mut self.query, queries, block("Queries", Focus::Queries));
        frame.render_widget(Paragraph::new(self.name.as_str()).block(block("Query name", Focus::Name)), name);
        frame.render_widget(Paragraph::new(self.runs.as_str()).block(block("Runs", Focus::Runs)), runs);
        self.editor.set_block(block(&format!("SQL on {}", self.selected_database()), Focus::Editor));
        frame.render_widget(&self.editor, sql);
        let output = Paragraph::new(self.result.as_str()).wrap(Wrap { trim: false });
        frame.render_widget(output.block(Block::bordered().title("Result")), result);
        frame.render_widget(Paragraph::new(HELP).style(Color::DarkGray), help);
    }
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Databases => Focus::Queries,
            Focus::Queries => Focus::Name,
            Focus::Name => Focus::Runs,
            Focus::Runs => Focus::Editor,
            Focus::Editor => Focus::Databases,
        }
    }
}

fn step(state: &mut ListState, code: KeyCode) {
    match code {
        KeyCode::Up => state.select_previous(),
        KeyCode::Down => state.select_next(),
        _ => {}
    }
}

fn edit(text: &mut String, code: KeyCode) {
    match code {
        KeyCode::Char(c) => text.push(c),
        KeyCode::Backspace => drop(text.pop()),
        _ => {}
    }
}

fn editor(sql: &str) -> TextArea<'static> {
    let mut editor = TextArea::new(sql.lines().map(String::from).collect());
    editor.set_cursor_line_style(Style::default());
    editor
}

fn saved_queries() -> Vec<String> {
    let files = fs::read_dir(QUERIES).into_iter().flatten().flatten().map(|file| file.path());
    let mut names: Vec<String> = files
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .filter_map(|path| Some(path.file_stem()?.to_str()?.to_string()))
        .collect();
    names.sort();
    names
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new().map_err(db::message)?;
    let mut terminal = ratatui::init();
    loop {
        while let Ok(text) = app.receiver.try_recv() {
            app.result = text;
        }
        terminal.draw(|frame| app.draw(frame))?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            let quit = key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL);
            if quit {
                break;
            }
            if key.kind == KeyEventKind::Press {
                app.on_key(key);
            }
        }
    }
    ratatui::restore();
    Ok(())
}
