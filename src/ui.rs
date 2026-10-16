//! Rendering.

use crate::app::App;
use openvxe::config::{Setting, MAX_DPI};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
};

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
}

/// Connection state shown in the header.
fn status(app: &App) -> (String, Color) {
    let pending = app.pending_writes();
    if app.devices.is_empty() {
        ("Dongle not connected: plug it in".into(), Color::Red)
    } else if app.asleep && app.config.is_none() {
        ("Waiting for the mouse: move it to wake it up".into(), Color::Yellow)
    } else if app.asleep {
        (
            format!("Mouse asleep: {} change(s) pending, move it", pending),
            Color::Yellow,
        )
    } else if app.config.is_none() {
        ("Reading configuration...".into(), Color::Yellow)
    } else if pending > 0 {
        (format!("Applying {} change(s)...", pending), Color::Yellow)
    } else {
        ("Ready".into(), Color::Green)
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    // Main container
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content
            Constraint::Length(8), // Logs
        ])
        .split(f.area());

    // Header: device, sensor, firmware and connection state
    let mut device = String::from("VXE R1 SE+");
    if app.dpi_editable() {
        device.push_str(" (PAW3395SE)");
    }
    if let Some(fw) = &app.firmware {
        device.push_str(&format!(" fw {}", fw));
    }
    let (status, status_color) = status(app);
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " OpenVXE ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" | {} | ", device)),
        Span::styled(status, Style::default().fg(status_color)),
        Span::raw(" | 'q' to exit"),
    ]))
    .block(panel(""))
    .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Two columns: settings on the left, device status on the right
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    // Settings list, read from the mouse
    let active_stage = app.config.as_ref().map(|c| c.active_stage());
    let items: Vec<ListItem> = if app.items.is_empty() {
        vec![ListItem::new("Waiting for the configuration...").style(Style::default().fg(Color::DarkGray))]
    } else {
        app.items
            .iter()
            .map(|item| {
                let mut text = item.to_string();
                if matches!(item, Setting::DpiStage { index, .. } if Some(*index) == active_stage) {
                    text.push_str("  < active");
                }
                ListItem::new(text).style(Style::default().fg(Color::White))
            })
            .collect()
    };
    let list = List::new(items)
        .block(panel(" Configuration (Use Arrow Keys) "))
        .highlight_style(
            Style::default()
                .bg(Color::White)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" > ");
    f.render_stateful_widget(list, main_chunks[0], &mut app.state);

    // Device status panel
    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    // DPI gauge: active stage
    let (dpi_percent, dpi_label) = match app.active_dpi() {
        Some((dpi, stage)) => (
            (dpi as f64 / MAX_DPI as f64 * 100.0).min(100.0) as u16,
            format!("{} DPI (stage {})", dpi, stage + 1),
        ),
        None => (0, "--".to_string()),
    };
    let dpi_gauge = Gauge::default()
        .block(panel(" DPI Level "))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .percent(dpi_percent)
        .label(dpi_label);
    f.render_widget(dpi_gauge, status_chunks[0]);

    // Battery gauge
    let (bat_percent, bat_color, bat_label) = match app.battery {
        None => (0, Color::DarkGray, "Waiting...".to_string()),
        Some((percent, charging)) => {
            let color = match percent {
                _ if charging => Color::Cyan,
                0..=20 => Color::Red,
                21..=50 => Color::Yellow,
                _ => Color::Green,
            };
            let label = format!("{}%{}", percent, if charging { " (charging)" } else { "" });
            (percent, color, label)
        }
    };
    let bat_gauge = Gauge::default()
        .block(panel(" Battery Level "))
        .gauge_style(Style::default().fg(bat_color).bg(Color::DarkGray))
        .percent(bat_percent.into())
        .label(bat_label);
    f.render_widget(bat_gauge, status_chunks[1]);

    // Event log
    let log_lines: Vec<Line> = app.logs.iter().map(|l| Line::from(l.as_str())).collect();
    f.render_widget(Paragraph::new(log_lines).block(panel(" Event Log ")), chunks[2]);
}
