/// The heap inspector — a TUI for browsing the MOOF object graph.
///
/// Navigate objects, environments, cons lists, bytecode chunks.
/// Enter to drill in, Backspace to go back, q to quit.

use std::io;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::*,
};

use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;

/// What we're looking at in the inspector.
#[derive(Clone)]
enum View {
    /// Browsing a specific value
    Value(Value),
    /// Browsing the heap by index (offset into objects list)
    HeapList { offset: usize, selected: usize },
}

pub fn run_inspector(heap: &Heap, initial: Option<Value>) -> io::Result<()> {
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let initial_view = match initial {
        Some(val) => View::Value(val),
        None => View::HeapList { offset: 0, selected: 0 },
    };

    let mut history: Vec<View> = Vec::new();
    let mut current = initial_view;

    loop {
        terminal.draw(|frame| {
            let area = frame.area();

            // Title bar
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)])
                .split(area);

            let title = Block::default()
                .borders(Borders::ALL)
                .title(" MOOF Inspector ")
                .title_alignment(Alignment::Center);
            frame.render_widget(title, chunks[0]);

            match &current {
                View::Value(val) => {
                    render_value_view(frame, chunks[1], heap, *val);
                }
                View::HeapList { offset, selected } => {
                    render_heap_list(frame, chunks[1], heap, *offset, *selected);
                }
            }

            // Status bar
            let status = Paragraph::new(" [Enter] drill in  [Backspace] back  [q] quit  [h] heap list")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(status, chunks[2]);
        })?;

        // Input handling
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('h') => {
                        history.push(current.clone());
                        current = View::HeapList { offset: 0, selected: 0 };
                    }
                    KeyCode::Backspace => {
                        if let Some(prev) = history.pop() {
                            current = prev;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(target) = get_drill_target(heap, &current) {
                            history.push(current.clone());
                            current = View::Value(target);
                        }
                    }
                    KeyCode::Up => {
                        match &mut current {
                            View::HeapList { selected, .. } => {
                                if *selected > 0 { *selected -= 1; }
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Down => {
                        match &mut current {
                            View::HeapList { selected, offset } => {
                                let max = heap.len().saturating_sub(1);
                                if *selected + *offset < max { *selected += 1; }
                                // Scroll if selected goes off screen
                                if *selected > 30 {
                                    *offset += 1;
                                    *selected -= 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn render_value_view(frame: &mut Frame, area: Rect, heap: &Heap, val: Value) {
    let mut lines: Vec<Line> = Vec::new();

    match val {
        Value::Nil => {
            lines.push(Line::from(vec![
                Span::styled("nil", Style::default().fg(Color::DarkGray)),
            ]));
        }
        Value::True => lines.push(Line::from("true")),
        Value::False => lines.push(Line::from("false")),
        Value::Integer(n) => {
            lines.push(Line::from(vec![
                Span::styled("Integer", Style::default().fg(Color::Cyan)),
                Span::raw(format!("  {}", n)),
            ]));
        }
        Value::Float(f) => {
            lines.push(Line::from(vec![
                Span::styled("Float", Style::default().fg(Color::Cyan)),
                Span::raw(format!("  {}", f)),
            ]));
        }
        Value::Symbol(id) => {
            lines.push(Line::from(vec![
                Span::styled("Symbol", Style::default().fg(Color::Yellow)),
                Span::raw(format!("  '{}", heap.symbol_name(id))),
            ]));
        }
        Value::Object(id) => {
            match heap.get(id) {
                HeapObject::Cons { car, cdr } => {
                    lines.push(Line::from(vec![
                        Span::styled("Cons", Style::default().fg(Color::Green).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(""));
                    lines.push(Line::from(vec![
                        Span::styled("  car: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *car)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  cdr: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *cdr)),
                    ]));

                    // Also show as list if it's a proper list
                    let elems = heap.list_to_vec(val);
                    if elems.len() > 1 {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled("  as list:", Style::default().fg(Color::DarkGray))));
                        for (i, elem) in elems.iter().enumerate().take(20) {
                            lines.push(Line::from(format!("    [{}] {}", i, format_value_short(heap, *elem))));
                        }
                        if elems.len() > 20 {
                            lines.push(Line::from(format!("    ... ({} total)", elems.len())));
                        }
                    }
                }
                HeapObject::MoofString(s) => {
                    lines.push(Line::from(vec![
                        Span::styled("String", Style::default().fg(Color::Green).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(format!("  \"{}\"", s)));
                    lines.push(Line::from(format!("  length: {}", s.chars().count())));
                }
                HeapObject::GeneralObject { parent, slots, handlers } => {
                    lines.push(Line::from(vec![
                        Span::styled("Object", Style::default().fg(Color::Green).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  parent: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *parent)),
                    ]));
                    lines.push(Line::from(""));

                    if !slots.is_empty() {
                        lines.push(Line::from(Span::styled("  slots:", Style::default().fg(Color::Yellow).bold())));
                        for (sym, val) in slots {
                            lines.push(Line::from(format!("    {}: {}",
                                heap.symbol_name(*sym), format_value_short(heap, *val))));
                        }
                        lines.push(Line::from(""));
                    }

                    if !handlers.is_empty() {
                        lines.push(Line::from(Span::styled("  handlers:", Style::default().fg(Color::Magenta).bold())));
                        for (sym, handler) in handlers {
                            lines.push(Line::from(format!("    {}: {}",
                                heap.symbol_name(*sym), format_value_short(heap, *handler))));
                        }
                    }
                }
                HeapObject::BytecodeChunk(chunk) => {
                    lines.push(Line::from(vec![
                        Span::styled("BytecodeChunk", Style::default().fg(Color::Green).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(format!("  {} bytes, {} constants", chunk.code.len(), chunk.constants.len())));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled("  constants:", Style::default().fg(Color::Cyan))));
                    for (i, c) in chunk.constants.iter().enumerate().take(20) {
                        lines.push(Line::from(format!("    [{}] {}", i, format_value_short(heap, *c))));
                    }
                }
                HeapObject::Lambda { params, body, def_env, source } => {
                    lines.push(Line::from(vec![
                        Span::styled("Lambda", Style::default().fg(Color::Blue).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  params: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *params)),
                    ]));
                    lines.push(Line::from(format!("  body chunk: #{}", body)));
                    lines.push(Line::from(format!("  def_env: #{}", def_env)));
                    lines.push(Line::from(vec![
                        Span::styled("  source: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *source)),
                    ]));
                }
                HeapObject::Operative { params, env_param, body, def_env, source } => {
                    lines.push(Line::from(vec![
                        Span::styled("Operative", Style::default().fg(Color::Red).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  params: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *params)),
                    ]));
                    lines.push(Line::from(format!("  env_param: {}", heap.symbol_name(*env_param))));
                    lines.push(Line::from(format!("  body chunk: #{}", body)));
                    lines.push(Line::from(format!("  def_env: #{}", def_env)));
                    lines.push(Line::from(vec![
                        Span::styled("  source: ", Style::default().fg(Color::Cyan)),
                        Span::raw(format_value_short(heap, *source)),
                    ]));
                }
                HeapObject::ForeignFunction { lib_name, func_name, arg_types, ret_type } => {
                    lines.push(Line::from(vec![
                        Span::styled("ForeignFunction", Style::default().fg(Color::Red).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(format!("  {}:{}", lib_name, func_name)));
                    lines.push(Line::from(format!("  ({}) -> {}", arg_types.join(", "), ret_type)));
                }
                HeapObject::Environment(env) => {
                    lines.push(Line::from(vec![
                        Span::styled("Environment", Style::default().fg(Color::Green).bold()),
                        Span::raw(format!("  #{}", id)),
                    ]));
                    lines.push(Line::from(format!("  parent: {:?}", env.parent.map(|p| format!("#{}", p)).unwrap_or("none".into()))));
                    lines.push(Line::from(format!("  {} bindings", env.bindings.len())));
                    lines.push(Line::from(""));

                    let mut bindings: Vec<_> = env.bindings.iter().collect();
                    bindings.sort_by_key(|(k, _)| *k);
                    for (sym, val) in bindings.iter().take(40) {
                        lines.push(Line::from(format!("    {}: {}",
                            heap.symbol_name(**sym), format_value_short(heap, **val))));
                    }
                    if bindings.len() > 40 {
                        lines.push(Line::from(format!("    ... ({} total)", bindings.len())));
                    }
                }
            }
        }
    }

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Value "))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_heap_list(frame: &mut Frame, area: Rect, heap: &Heap, offset: usize, selected: usize) {
    let visible = (area.height as usize).saturating_sub(2);
    let mut items: Vec<ListItem> = Vec::new();

    for i in offset..heap.len().min(offset + visible) {
        let obj = heap.get(i as u32);
        let type_name = match obj {
            HeapObject::Cons { .. } => "Cons",
            HeapObject::MoofString(_) => "String",
            HeapObject::GeneralObject { .. } => "Object",
            HeapObject::BytecodeChunk(_) => "Bytecode",
            HeapObject::Operative { .. } => "Operative",
            HeapObject::Lambda { .. } => "Lambda",
            HeapObject::Environment(_) => "Env",
            HeapObject::ForeignFunction { .. } => "FFI",
        };
        let summary = match obj {
            HeapObject::MoofString(s) => {
                let truncated: String = s.chars().take(40).collect();
                format!("\"{}\"", truncated)
            }
            HeapObject::GeneralObject { slots, handlers, .. } => {
                format!("{} slots, {} handlers", slots.len(), handlers.len())
            }
            HeapObject::Environment(env) => {
                format!("{} bindings", env.bindings.len())
            }
            HeapObject::BytecodeChunk(chunk) => {
                format!("{} bytes", chunk.code.len())
            }
            _ => String::new(),
        };

        let style = if i - offset == selected {
            Style::default().bg(Color::DarkGray).fg(Color::White)
        } else {
            Style::default()
        };

        items.push(ListItem::new(
            format!("#{:<6} {:<10} {}", i, type_name, summary)
        ).style(style));
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(
            format!(" Heap ({} objects) ", heap.len())
        ));
    frame.render_widget(list, area);
}

fn get_drill_target(heap: &Heap, view: &View) -> Option<Value> {
    match view {
        View::HeapList { offset, selected } => {
            let idx = (*offset + *selected) as u32;
            if (idx as usize) < heap.len() {
                Some(Value::Object(idx))
            } else {
                None
            }
        }
        View::Value(val) => {
            // Drill into the first interesting reference
            match val {
                Value::Object(id) => {
                    match heap.get(*id) {
                        HeapObject::Cons { car, .. } => Some(*car),
                        HeapObject::GeneralObject { parent, .. } => {
                            if *parent != Value::Nil { Some(*parent) } else { None }
                        }
                        HeapObject::Lambda { body, .. } => Some(Value::Object(*body)),
                        HeapObject::Operative { body, .. } => Some(Value::Object(*body)),
                        HeapObject::Environment(env) => {
                            env.parent.map(|p| Value::Object(p))
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        }
    }
}

fn format_value_short(heap: &Heap, val: Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::True => "true".to_string(),
        Value::False => "false".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'{}", heap.symbol_name(id)),
        Value::Object(id) => {
            match heap.get(id) {
                HeapObject::Cons { .. } => format!("(list) #{}", id),
                HeapObject::MoofString(s) => {
                    let truncated: String = s.chars().take(30).collect();
                    if s.len() > 30 { format!("\"{}...\"", truncated) }
                    else { format!("\"{}\"", truncated) }
                }
                HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
                HeapObject::BytecodeChunk(_) => format!("<bytecode #{}>", id),
                HeapObject::Lambda { .. } => format!("<lambda #{}>", id),
                HeapObject::Operative { .. } => format!("<operative #{}>", id),
                HeapObject::Environment(_) => format!("<env #{}>", id),
                HeapObject::ForeignFunction { lib_name, func_name, .. } => {
                    format!("<ffi {}:{}>", lib_name, func_name)
                }
            }
        }
    }
}
