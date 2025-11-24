use chrono::{DateTime, Utc};
use ratatui::{prelude::*, widgets::*};
use std::time::Instant;

use crate::config::Config;

/// 登录凭证类型
#[derive(Debug, Clone)]
pub enum Credential {
    Code { code: String, expires_at: String },
    Magic { url: String, expires_at: String },
}

/// 应用程序屏幕状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Dashboard,
    System,
}

/// 用户输入模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    None,
    Params {
        selected: usize,
        draft_ttl: u64,
        draft_uses: u32,
        draft_len: usize,
    },
    ConfirmQuit,
    ConfirmDeleteKey { index: usize },
    TokenDetails { index: usize },
    CopyChoice,
}

/// 应用程序主状态
pub struct AppState {
    pub message: String,
    pub cfg: Config,
    pub connected: bool,
    pub connect_error: Option<String>,
    pub last: Option<Credential>,
    pub last_code: Option<(String, String)>, // (code, expires_at)
    pub last_code_max_uses: Option<u32>,
    pub last_code_remaining: Option<u32>,
    pub last_code_uses: Option<u32>,
    pub last_code_disabled: Option<bool>,
    pub last_magic: Option<(String, String)>, // (url, expires_at)
    pub last_magic_remaining: Option<u32>,
    pub last_magic_disabled: Option<bool>,
    pub input_mode: InputMode,
    pub select_copy: bool,
    pub fingerprint: String,
    pub session_expires_at: Option<DateTime<Utc>>,
    pub screen: Screen,
    pub menu_index: usize,
    // System screen state
    pub system_tab: usize, // 0=tokens, 1=admin_keys
    pub tokens: Vec<crate::api::AdminTokenOut>,
    pub selected_token: usize,
    pub token_offset: usize,
    pub admin_keys: Vec<crate::api::AdminKeyOut>,
    pub selected_key: usize,
    pub key_offset: usize,
    // Toast
    pub toast: Option<String>,
    pub toast_deadline: Option<Instant>,
}

/// 主渲染函数
///
/// 根据应用程序状态渲染相应的用户界面
///
/// # Arguments
///
/// * `frame` - 终端渲染帧
/// * `app` - 应用程序状态
pub fn render(frame: &mut Frame, app: &AppState) {
    let size = frame.area();

    // 屏幕主体渲染（Home/System/Dashboard）
    if app.screen == Screen::Home {
        render_home(frame, app, size);
        // Home 也需要覆盖层（Toast / 确认退出）
        render_overlays(frame, app, size);
        return;
    } else if app.screen == Screen::System {
        render_system(frame, app, size);
        // System 也需要覆盖层（Toast / 确认退出）
        render_overlays(frame, app, size);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // 顶部多加一行空白
            Constraint::Min(8),    // 主内容
            Constraint::Length(1), // 底部帮助（减少两行）
        ])
        .split(size);

    // Header
    let status = if app.connected {
        "已连接"
    } else {
        "未连接"
    };
    let header_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(3)])
        .split(chunks[0]);
    // 第一行空白自动保留
    let _fp = if app.fingerprint.is_empty() {
        "<未配置>".to_string()
    } else {
        app.fingerprint.clone()
    };
    let session_info = app
        .session_expires_at
        .map(|dt| format_expire(&dt.to_rfc3339()))
        .unwrap_or_else(|| "--".into());
    let header_line = format!(
        "服务: {}    状态: {}    会话有效期至: {}",
        app.cfg.api_base_url, status, session_info,
    );
    let header = Paragraph::new(Line::from(header_line).style(if app.connected {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    }))
    .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, header_rows[1]);

    // Main split
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    // Left: Controls + Current Config Summary
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // 操作（3 行 + 边框）
            Constraint::Length(3), // 选择复制开关（1 行 + 边框）
            Constraint::Min(8),    // 配置摘要（上下 2 块）
        ])
        .split(main[0]);

    // 操作菜单：表格左右两列，严格对齐
    let ops_rows = vec![
        Row::new(vec![
            Cell::from("[G] 生成 Code"),
            Cell::from("[M] 生成 Magic URL"),
        ]),
        Row::new(vec![
            Cell::from("[Y] 复制凭证"),
            Cell::from("[S] 保存到文件"),
        ]),
        Row::new(vec![
            Cell::from("[C] 配置 Code 参数"),
            Cell::from("[R] 重试连接"),
        ]),
    ];
    let ops_table = Table::new(
        ops_rows,
        [Constraint::Percentage(50), Constraint::Percentage(50)],
    )
    .block(Block::default().title("操作").borders(Borders::ALL))
    .column_spacing(1);
    frame.render_widget(ops_table, left_chunks[0]);

    // 选择复制开关（独立一行，显示在操作菜单正下方）
    let toggle_text = format!(
        "允许选择复制 (V): {}",
        if app.select_copy { "开" } else { "关" }
    );
    let toggle = Paragraph::new(toggle_text)
        .block(Block::default().borders(Borders::ALL).title("选择复制"))
        .alignment(Alignment::Left);
    frame.render_widget(toggle, left_chunks[1]);

    // 底部：分为“新 Code 默认配置”与“最近一次 Code 配置”上下两块
    let left_bottom = Layout::default()
        .direction(Direction::Vertical)
        // 上：新 Code 默认配置（3 行 + 边框 = 5），下：最近一次 Code（3 行 + 边框 = 5）
        .constraints([Constraint::Length(5), Constraint::Length(5)])
        .split(left_chunks[2]);

    // 新默认配置
    let default_lines = vec![
        Line::from(format!("过期时间: {} s", app.cfg.ttl_secs)),
        Line::from(format!("可用次数: {}", app.cfg.max_uses)),
        Line::from(format!("长度: {} 位", app.cfg.length)),
    ];
    let default_para = Paragraph::new(Text::from(default_lines))
        .block(
            Block::default()
                .title("新 Code 默认配置")
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: true });
    frame.render_widget(default_para, left_bottom[0]);

    // 最近一次 Code 信息
    let recent_lines: Vec<Line> = if let Some((code, _)) = &app.last_code {
        let max_uses = app.last_code_max_uses.unwrap_or(app.cfg.max_uses);
        let used = app.last_code_uses.unwrap_or(0);
        let remaining = app
            .last_code_remaining
            .unwrap_or_else(|| max_uses.saturating_sub(used))
            .min(max_uses);
        let status_line = if app.last_code_disabled.unwrap_or(false) || remaining == 0 {
            "状态: 已失效".to_string()
        } else {
            "状态: 可用".to_string()
        };
        vec![
            Line::from(format!("Code: {}", code)),
            Line::from(format!(
                "可用次数: {}/{}  已用: {}",
                remaining, max_uses, used
            )),
            Line::from(format!("离过期还剩: {} s", remain_seconds(app).max(0))),
            Line::from(status_line),
        ]
    } else {
        vec![Line::from("暂无最近一次 Code 记录")]
    };
    let recent_para = Paragraph::new(Text::from(recent_lines))
        .block(Block::default().title("最近一次 Code 信息").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(recent_para, left_bottom[1]);

    // Right: 凭证 + 消息（对齐要求：
    // 凭证：上与“操作”对齐，下与“新 Code 默认配置”对齐 → 高度=5+3+5=13
    // 消息：下边与“最近一次 Code 配置”对齐 → 高度=5
    // 其余为底部空白）
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(13),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(main[1]);

    // 凭证分上下布局：上 Code，下 Magic URL；中间 1 行空白，确保两者同时可见
    let cred_area = right_chunks[0];
    frame.render_widget(
        Block::default().title("凭证").borders(Borders::ALL),
        cred_area,
    );
    let inner = Rect {
        x: cred_area.x + 1,
        y: cred_area.y + 1,
        width: cred_area.width.saturating_sub(2),
        height: cred_area.height.saturating_sub(2),
    };
    let spacer = 1u16;
    let top_h = (inner.height.saturating_sub(spacer)) / 2;
    let bot_h = inner.height.saturating_sub(spacer + top_h);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Length(spacer),
            Constraint::Length(bot_h),
        ])
        .split(inner);

    // 上：Code（两行：Code 与 有效截止时间）
    let code_lines: Vec<Line> = if let Some((code, exp)) = &app.last_code {
        vec![
            Line::from("Code:"),
            Line::from(code.clone()),
            Line::from("有效截止时间:"),
            Line::from(format_expire(exp)),
        ]
    } else {
        vec![Line::from("Code: （无）")]
    };
    let code_para = Paragraph::new(Text::from(code_lines)).wrap(Wrap { trim: true });
    frame.render_widget(code_para, parts[0]);

    // 下：Magic URL（两行：URL 与 有效截止时间）
    let magic_lines: Vec<Line> = if let Some((url, exp)) = &app.last_magic {
        let status = if app.last_magic_disabled.unwrap_or(false)
            || !super_valid(exp)
        {
            "已被使用/已失效"
        } else {
            "有效"
        };
        vec![
            Line::from("Magic URL:"),
            Line::from(url.clone()),
            Line::from("有效截止时间:"),
            Line::from(format_expire(exp)),
            Line::from(format!("有效状态：{}", status)),
        ]
    } else {
        vec![Line::from("Magic URL: （无）")]
    };
    let magic_para = Paragraph::new(Text::from(magic_lines)).wrap(Wrap { trim: true });
    frame.render_widget(magic_para, parts[2]);

    // 消息：逐行显示
    let mut msg_lines: Vec<Line> = app
        .message
        .lines()
        .map(|s| Line::from(s.to_string()))
        .collect();
    if !app.connected {
        if let Some(e) = &app.connect_error {
            msg_lines.push(Line::from(format!("连接错误：{}", e)));
        }
    }
    let msg_para = Paragraph::new(Text::from(msg_lines))
        .block(Block::default().title("消息").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(msg_para, right_chunks[1]);

    // Footer help
    let help = Paragraph::new("[Q] 退出  [G/M] 生成凭证  [Y] 复制  [S] 保存  [C] 参数设置  [H] 返回首页")
        .alignment(Alignment::Center);
    frame.render_widget(help, chunks[2]);

    // Toast + 确认退出覆盖层（Dashboard）
    render_overlays(frame, app, size);

    // 二级参数面板（平铺色块）
    match app.input_mode {
        InputMode::Params {
            selected,
            draft_ttl,
            draft_uses,
            draft_len,
        } => {
            let area = centered_rect_fixed(56, 11, size);
            frame.render_widget(Clear, area);
            let outer = Block::default()
                .title("参数设置（↑↓ 切换  ←→ 调整  Enter 保存  Esc 取消）")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue));
            frame.render_widget(outer, area);

            let inner = Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width - 2,
                height: area.height - 2,
            };
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .split(inner);

            let labels = ["过期时间 (TTL)", "可用次数 (max_uses)", "长度 (25..64)"];
            let values = [
                format!("{} 秒", draft_ttl),
                format!("{}", draft_uses),
                format!("{} 位", draft_len),
            ];
            for (i, row) in rows.iter().take(3).enumerate() {
                let highlighted = i == selected;
                if highlighted {
                    // 背景色块
                    let fill = Block::default().style(Style::default().bg(Color::Cyan));
                    frame.render_widget(fill, *row);
                }
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(*row);
                let style = if highlighted {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                // 纵向居中：将显示区域压缩为 1 行并置于 row 中线
                // 垂直居中：在 3 行高度内的中间行渲染
                let label_area = Rect {
                    x: cols[0].x,
                    y: cols[0].y + (cols[0].height.saturating_sub(1)) / 2,
                    width: cols[0].width,
                    height: 1,
                };
                let value_area = Rect {
                    x: cols[1].x,
                    y: cols[1].y + (cols[1].height.saturating_sub(1)) / 2,
                    width: cols[1].width,
                    height: 1,
                };
                let label = Paragraph::new(labels[i])
                    .style(style)
                    .alignment(Alignment::Center);
                let value = Paragraph::new(values[i].clone())
                    .style(style)
                    .alignment(Alignment::Center);
                frame.render_widget(label, label_area);
                frame.render_widget(value, value_area);
            }
        }
        InputMode::CopyChoice => {
            let area = centered_rect_fixed(42, 9, size);
            frame.render_widget(Clear, area);
            let block = Block::default().title("复制哪个？").borders(Borders::ALL);
            frame.render_widget(block, area);
            let inner = Rect {
                x: area.x + 1,
                y: area.y + 1,
                width: area.width - 2,
                height: area.height - 2,
            };
            let lines = vec![
                "  [1] 复制 Code",
                "  [2] 复制 Magic URL",
                "  [3] 同时复制两者",
                "  [Esc] 取消",
            ];
            // 垂直居中 4 行，左对齐
            let start_y = inner.y + (inner.height.saturating_sub(lines.len() as u16)) / 2;
            for (i, text) in lines.into_iter().enumerate() {
                let r = Rect {
                    x: inner.x,
                    y: start_y + i as u16,
                    width: inner.width,
                    height: 1,
                };
                let p = Paragraph::new(text).alignment(Alignment::Left);
                frame.render_widget(p, r);
            }
        }
        InputMode::ConfirmQuit => { /* 覆盖层统一在 render_overlays 中绘制，这里占位避免重复 */ }
        _ => {}
    }
}

/// 渲染主页界面
///
/// 显示应用程序的主页，包括 Logo 和主菜单
///
/// # Arguments
///
/// * `frame` - 终端渲染帧
/// * `app` - 应用程序状态
/// * `size` - 可用渲染区域
fn render_home(frame: &mut Frame, app: &AppState, size: Rect) {
    let outer = Block::default().borders(Borders::NONE);
    frame.render_widget(outer, size);

    // 布局：顶部 ASCII Logo + 菜单 + 底部消息
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(8), Constraint::Length(3)])
        .split(size);

    // ASCII Logo 居中（明确写成 GATEWAY）
    let logo = r#"
    ██████╗  █████╗ ████████╗███████╗██╗    ██╗ █████╗ ███╗   ██╗
    ██╔══██╗██╔══██╗╚══██╔══╝██╔════╝██║    ██║██╔══██╗████╗  ██║
    ██████╔╝███████║   ██║   █████╗  ██║ █╗ ██║███████║██╔██╗ ██║
    ██╔══██╗██╔══██║   ██║   ██╔══╝  ██║███╗██║██╔══██║██║╚██╗██║
    ██████╔╝██║  ██║   ██║   ███████╗╚███╔███╔╝██║  ██║██║ ╚████║
    ╚═════╝ ╚═╝  ╚═╝   ╚═╝   ╚══════╝ ╚══╝╚══╝ ╚═╝  ╚═╝╚═╝  ╚═══╝
                                        —— GateWay
    "#;
    let logo_para = Paragraph::new(logo)
        .alignment(Alignment::Center)
        .block(Block::default());
    frame.render_widget(logo_para, chunks[0]);

    // 菜单
    let items = [
        "生成 Web 端登录凭证",
        "重置 TUI 验证密钥",
        "系统管理",
        "退出 TUI",
    ];
    let inner = centered_rect_fixed(48, (items.len() as u16) + 4, chunks[1]);
    frame.render_widget(Clear, inner);
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let style = if i == app.menu_index {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(*text)).style(style)
        })
        .collect();
    let list = List::new(list_items)
        .block(
            Block::default()
                .title("选择操作（↑↓ 切换，Enter 确认）")
                .borders(Borders::ALL),
        )
        .highlight_symbol("▶ ");
    frame.render_widget(list, inner);

    // 底部消息
    let msg = Paragraph::new(app.message.as_str())
        .alignment(Alignment::Center)
        .block(Block::default());
    frame.render_widget(msg, chunks[2]);

    // Home 屏幕也渲染覆盖层（Toast/确认退出）——为稳妥起见保留
    // 实际覆盖层由顶层 render 调用 render_overlays 绘制，这里无需重复
}

/// 渲染系统管理界面
///
/// 显示系统管理界面，包括令牌和管理员密钥管理
///
/// # Arguments
///
/// * `frame` - 终端渲染帧
/// * `app` - 应用程序状态
/// * `size` - 可用渲染区域
pub fn render_system(frame: &mut Frame, app: &AppState, size: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(1)])
        .split(size);
    let header = Paragraph::new("系统管理  [Tab] 切换 令牌/密钥  ↑↓ 移动  [Enter] 查看令牌详情  [Del] 删除密钥  [R] 刷新  [H] 返回首页")
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, layout[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[1]);

    // 左侧：令牌表（带选中高亮）
    let mut token_rows: Vec<Row> = Vec::new();
    token_rows.push(Row::new(vec![
        format!("{:<22}", "Token"),
        format!("{:^8}", "Enabled"),
        format!("{:^22}", "Expires"),
        format!("{:>12}", "Spent"),
    ]).style(Style::default().fg(Color::Yellow)));
    const VISIBLE_ROWS: usize = 10;
    for (i, t) in app.tokens.iter().enumerate().skip(app.token_offset).take(VISIBLE_ROWS) {
        let token_disp = if t.token.len() > 20 { format!("{}…{}", &t.token[..10], &t.token[t.token.len().saturating_sub(5)..]) } else { t.token.clone() };
        let en = if t.enabled { "Y" } else { "N" };
        let expires = t.expires_at.clone().unwrap_or_else(|| "--".into());
        let spent = format!("{:.2}", t.amount_spent);
        let mut row = Row::new(vec![
            format!("{:<22}", token_disp),
            format!("{:^8}", en),
            format!("{:^22}", expires),
            format!("{:>12}", spent),
        ]);
        if app.system_tab == 0 && (app.token_offset + i) == app.selected_token {
            row = row.style(Style::default().fg(Color::Black).bg(Color::Cyan));
        }
        token_rows.push(row);
    }
    let tokens_table = Table::new(token_rows, [Constraint::Length(22), Constraint::Length(8), Constraint::Length(22), Constraint::Length(12)])
        .block(Block::default().title(if app.system_tab == 0 { "令牌（选中）" } else { "令牌" }).borders(Borders::ALL));
    frame.render_widget(tokens_table, body[0]);

    // Admin keys table（带选中高亮 + 当前密钥标记）
    let mut key_rows: Vec<Row> = Vec::new();
    // 动态计算 Comment 列宽以右对齐（内宽=面板宽-左右边框；列间距默认1，共两处）
    let inner_w = body[1].width.saturating_sub(2);
    let spacing = 1u16;
    let fixed = 26u16 + 8u16; // Fingerprint + Current 固定宽
    let gaps = spacing * 2;   // 三列 → 两个间隔
    let comment_w = inner_w.saturating_sub(fixed + gaps).max(1);

    key_rows.push(
        Row::new(vec![
            format!("{:<26}", "Fingerprint"),
            format!("{:^8}", "Current"),
            format!("{:>w$}", "Comment", w = comment_w as usize),
        ])
        .style(Style::default().fg(Color::Yellow)),
    );
    for (i, k) in app
        .admin_keys
        .iter()
        .enumerate()
        .skip(app.key_offset)
        .take(VISIBLE_ROWS)
    {
        let fp = if k.fingerprint.len() > 24 {
            format!("{}…", &k.fingerprint[..24])
        } else {
            k.fingerprint.clone()
        };
        let cur = if k.fingerprint == app.fingerprint { "Y" } else { "" };
        let c = k.comment.clone().unwrap_or_default();
        let mut row = Row::new(vec![
            format!("{:<26}", fp),
            format!("{:^8}", cur),
            format!("{:>w$}", c, w = comment_w as usize),
        ]);
        if app.system_tab == 1 && (app.key_offset + i) == app.selected_key {
            row = row.style(Style::default().fg(Color::Black).bg(Color::Cyan));
        }
        key_rows.push(row);
    }
    let keys_table = Table::new(key_rows, [
        Constraint::Length(26),
        Constraint::Length(8),
        Constraint::Length(comment_w),
    ])
        .block(Block::default().title(if app.system_tab == 1 { "管理员密钥（选中）" } else { "管理员密钥" }).borders(Borders::ALL));
    frame.render_widget(keys_table, body[1]);

    let footer = Paragraph::new("[Tab] 切换  [Enter] 详情(令牌)  [Del] 删除(密钥)  [R] 刷新  [H] 返回首页").alignment(Alignment::Center);
    frame.render_widget(footer, layout[2]);
}

/// 渲染覆盖层界面
///
/// 渲染各种弹窗和提示信息，如确认对话框、Toast 通知等
///
/// # Arguments
///
/// * `frame` - 终端渲染帧
/// * `app` - 应用程序状态
/// * `size` - 可用渲染区域
fn render_overlays(frame: &mut Frame, app: &AppState, size: Rect) {
    // Toast overlay（右上角，全局）
    if let Some(text) = &app.toast {
        let w = (text.len().min(40) as u16) + 6;
        let h = 3u16;
        let area = Rect {
            x: size.x + size.width.saturating_sub(w + 2),
            y: size.y + 1,
            width: w,
            height: h,
        };
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .title("提示")
            .border_style(Style::default().fg(Color::Green));
        frame.render_widget(block, area);
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width - 2,
            height: area.height - 2,
        };
        let p = Paragraph::new(text.as_str()).alignment(Alignment::Center);
        frame.render_widget(p, inner);
    }

    // 确认退出弹窗（全局）
    if matches!(app.input_mode, InputMode::ConfirmQuit) {
        let area = centered_rect_fixed(34, 5, size);
        frame.render_widget(Clear, area);
        let block = Block::default().title("确认").borders(Borders::ALL);
        frame.render_widget(block, area);
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width - 2,
            height: area.height - 2,
        };
        let text_area = Rect {
            x: inner.x,
            y: inner.y + (inner.height.saturating_sub(1)) / 2,
            width: inner.width,
            height: 1,
        };
        let text = Paragraph::new("确认退出？ [Y] 是  [N] 否").alignment(Alignment::Center);
        frame.render_widget(text, text_area);
    }

    // 确认删除管理员密钥
    if let InputMode::ConfirmDeleteKey { index } = app.input_mode {
        let area = centered_rect_fixed(44, 5, size);
        frame.render_widget(Clear, area);
        let block = Block::default().title("确认删除密钥").borders(Borders::ALL);
        frame.render_widget(block, area);
        let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width - 2, height: area.height - 2 };
        let text_area = Rect { x: inner.x, y: inner.y + (inner.height.saturating_sub(2))/2, width: inner.width, height: 2 };
        let fp = app.admin_keys.get(index).map(|k| k.fingerprint.as_str()).unwrap_or("?");
        let lines = vec![
            Line::from(format!("删除 Fingerprint: {}？", fp)),
            Line::from("[Y] 是  [N] 否"),
        ];
        let p = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
        frame.render_widget(p, text_area);
    }

    // 令牌详情弹窗
    if let InputMode::TokenDetails { index } = app.input_mode {
        if let Some(t) = app.tokens.get(index) {
            let area = centered_rect_fixed(64, 12, size);
            frame.render_widget(Clear, area);
            let block = Block::default().title("令牌详情").borders(Borders::ALL);
            frame.render_widget(block, area);
            let inner = Rect { x: area.x + 1, y: area.y + 1, width: area.width - 2, height: area.height - 2 };
            let mut lines: Vec<Line> = Vec::new();
            lines.push(Line::from(format!("Token: {}", t.token)));
            lines.push(Line::from(format!("Enabled: {}", if t.enabled {"Y"} else {"N"})));
            if let Some(exp) = &t.expires_at { lines.push(Line::from(format!("Expires: {}", exp))); }
            lines.push(Line::from(format!("Created: {}", t.created_at)));
            if let Some(ma) = t.max_amount { lines.push(Line::from(format!("Max Amount: {:.2}", ma))); }
            lines.push(Line::from(format!("Amount Spent: {:.2}", t.amount_spent)));
            // Tokens (separate lines, English)
            lines.push(Line::from(format!("Prompt Tokens: {}", t.prompt_tokens_spent)));
            lines.push(Line::from(format!("Completion Tokens: {}", t.completion_tokens_spent)));
            lines.push(Line::from(format!("Total Tokens: {}", t.total_tokens_spent)));
            let p = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true });
            frame.render_widget(p, inner);
        }
    }
}

/// 创建居中的矩形区域
///
/// 在给定的区域内创建一个指定尺寸的居中矩形
///
/// # Arguments
///
/// * `width` - 所需宽度
/// * `height` - 所需高度
/// * `r` - 父级区域
///
/// # Returns
///
/// 居中的矩形区域
fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let w = width.min(r.width.saturating_sub(2));
    let h = height.min(r.height.saturating_sub(2));
    let x = r.x + (r.width.saturating_sub(w)) / 2;
    let y = r.y + (r.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}


/// 计算凭证剩余有效秒数
///
/// 根据应用程序状态计算当前登录凭证的剩余有效时间
///
/// # Arguments
///
/// * `app` - 应用程序状态
///
/// # Returns
///
/// 剩余有效秒数，如果已失效则返回 0
fn remain_seconds(app: &AppState) -> i64 {
    if app.last_code_disabled.unwrap_or(false) {
        return 0;
    }
    if let Some(rem) = app.last_code_remaining {
        if rem == 0 {
            return 0;
        }
    }
    if let Some((_, expires)) = &app.last_code {
        if let Ok(dt) = DateTime::parse_from_rfc3339(expires) {
            let rem = dt.with_timezone(&Utc) - Utc::now();
            return rem.num_seconds();
        }
    }
    app.cfg.ttl_secs as i64
}

/// 格式化过期时间显示
///
/// 将 RFC3339 格式的时间字符串转换为可读的北京时间格式
///
/// # Arguments
///
/// * `expires` - RFC3339 格式的时间字符串
///
/// # Returns
///
/// 格式化后的时间字符串
fn format_expire(expires: &str) -> String {
    // 将 RFC3339 转为北京时间可读格式
    if let Ok(dt) = DateTime::parse_from_rfc3339(expires) {
        if let Some(tz) = chrono::FixedOffset::east_opt(8 * 3600) {
            let bj = dt.with_timezone(&tz);
            return bj.format("%Y-%m-%d %H:%M:%S").to_string();
        }
        return dt
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
    }
    expires.to_string()
}

/// 检查时间是否有效
///
/// 检查给定的 RFC3339 时间是否在当前时间之后
///
/// # Arguments
///
/// * `expires` - RFC3339 格式的时间字符串
///
/// # Returns
///
/// 如果有效则返回 true，否则返回 false
fn super_valid(expires: &str) -> bool {
    if let Ok(dt) = DateTime::parse_from_rfc3339(expires) {
        return dt.with_timezone(&Utc) > Utc::now();
    }
    false
}
