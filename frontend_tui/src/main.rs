use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64_STANDARD;
use chrono::{DateTime, Utc};
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ed25519_dalek::{Signer, SigningKey};
use ratatui::prelude::CrosstermBackend;

mod api;
mod config;
mod session;
mod ui;

use crate::api::{
    add_admin_key, connectivity_check, create_code, delete_admin_key, fetch_code_status,
    list_admin_keys, list_tokens, request_challenge, verify_challenge,
};
use crate::config::{
    Config, ensure_config_interactive, fingerprint as cfg_fingerprint, load_signing_key,
    save_config,
};
use crate::session::{SessionCache, clear_cache, load_cache, save_cache};
use crate::ui::{AppState, Credential, InputMode, render};

const SIGNING_PREFIX: &[u8] = b"gateway-auth:";
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(1);
const SYSTEM_VISIBLE_ROWS: usize = 10;

type TermResult<T> = Result<T, Box<dyn std::error::Error>>;

/// 会话状态结构
///
/// 存储管理员会话的令牌和过期时间
#[derive(Clone)]
struct SessionState {
    token: String,
    expires_at: DateTime<Utc>,
}

/// 解析 Magic URL
///
/// 将相对路径或完整 URL 转换为完整的 Magic URL
///
/// # Arguments
///
/// * `cfg` - 包含基础 URL 的配置
/// * `raw` - 原始 URL 或路径
///
/// # Returns
///
/// 完整的 Magic URL
fn resolve_magic_url(cfg: &Config, raw: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!(
            "{}/{}",
            cfg.api_base_url.trim_end_matches('/'),
            raw.trim_start_matches('/')
        )
    }
}

/// 解析过期时间字符串
///
/// 将 RFC3339 格式的时间字符串解析为 UTC 时间
///
/// # Arguments
///
/// * `expires` - RFC3339 格式的时间字符串
///
/// # Returns
///
/// * `Some(DateTime<Utc>)` - 成功解析的 UTC 时间
/// * `None` - 解析失败
fn parse_expiry(expires: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(expires)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// 检查凭证是否有效
///
/// 检查给定的过期时间是否在当前时间之后
///
/// # Arguments
///
/// * `expires` - RFC3339 格式的过期时间
///
/// # Returns
///
/// 如果凭证仍然有效则返回 true
fn credential_valid(expires: &str) -> bool {
    parse_expiry(expires)
        .map(|dt| dt > Utc::now())
        .unwrap_or(false)
}

/// 复制文本到系统剪贴板
///
/// 尝试使用系统剪贴板，如果失败则使用 OSC52 转义序列作为回退方案
///
/// # Arguments
///
/// * `text` - 要复制的文本
///
/// # Returns
///
/// * `Ok(())` - 复制成功
/// * `Err(String)` - 复制失败的错误信息
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
        Ok(()) => return Ok(()),
        Err(e) => {
            let b64 = B64_STANDARD.encode(text.as_bytes());
            let osc = format!("\x1b]52;c;{}\x07", b64);
            let mut out = std::io::stdout().lock();
            if out.write_all(osc.as_bytes()).is_ok() && out.flush().is_ok() {
                return Ok(());
            }
            return Err(format!("系统剪贴板失败且 OSC52 回退失败：{}", e));
        }
    }
}

/// 保存凭证到文件
///
/// 将最后一次生成的凭证保存到本地文件
///
/// # Arguments
///
/// * `c` - 要保存的凭证
///
/// # Returns
///
/// * `Ok(String)` - 保存文件的路径
/// * `Err(String)` - 保存失败的错误信息
fn save_last_to_file(c: &Credential) -> Result<String, String> {
    let p = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("last_credential.txt");
    let s = match c {
        Credential::Code { code, expires_at } => {
            format!("Code: {}\nExpires: {}\n", code, expires_at)
        }
        Credential::Magic { url, expires_at } => {
            format!("Magic URL: {}\nExpires: {}\n", url, expires_at)
        }
    };
    std::fs::write(&p, s).map_err(|e| e.to_string())?;
    Ok(p.display().to_string())
}

/// 初始化终端
///
/// 设置终端为原始模式并创建 ratatui 终端实例
///
/// # Returns
///
/// * `Ok(Terminal)` - 初始化成功的终端
/// * `Err(Box<dyn Error>)` - 初始化失败的错误
fn init_terminal() -> TermResult<ratatui::Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = ratatui::Terminal::new(backend)?;
    Ok(terminal)
}

/// 恢复终端状态
///
/// 恢复终端到正常模式，退出备用屏幕并禁用原始模式
///
/// # Arguments
///
/// * `terminal` - 要恢复的终端实例
///
/// # Returns
///
/// 恢复结果
fn restore_terminal(mut terminal: ratatui::Terminal<CrosstermBackend<Stdout>>) -> TermResult<()> {
    disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// 加载缓存的会话
///
/// 从本地缓存加载会话状态，如果会话已过期则清理缓存
///
/// # Returns
///
/// * `Some(SessionState)` - 有效的会话状态
/// * `None` - 无有效会话或会话已过期
fn load_cached_session() -> Option<SessionState> {
    if let Some(cache) = load_cache() {
        if let Some(exp) = cache.expires_at() {
            if exp > Utc::now() {
                return Some(SessionState {
                    token: cache.token,
                    expires_at: exp,
                });
            }
        }
        clear_cache();
    }
    None
}

/// 检查是否为认证错误
///
/// 根据错误信息判断是否为身份认证相关的错误
///
/// # Arguments
///
/// * `err` - 错误信息字符串
///
/// # Returns
///
/// 如果是认证错误则返回 true
fn is_auth_error(err: &str) -> bool {
    err.contains("认证") || err.contains("未授权") || err.contains("token")
}

/// 确保管理员会话有效
///
/// 检查并维护管理员会话的有效性，包括本地缓存和远端验证
/// 如果会话无效或不存在，则发起新的挑战-应答认证流程
///
/// # Arguments
///
/// * `cfg` - 应用程序配置
/// * `signing_key` - 管理员签名私钥
/// * `fingerprint` - 管理员公钥指纹
/// * `session_state` - 可变的会话状态引用
///
/// # Returns
///
/// * `Ok(())` - 会话有效或成功建立新会话
/// * `Err(String)` - 会话验证或建立失败
fn ensure_session(
    cfg: &Config,
    signing_key: &SigningKey,
    fingerprint: &str,
    session_state: &mut Option<SessionState>,
) -> Result<(), String> {
        if let Some(state) = session_state.clone() {
        if state.expires_at > Utc::now() {
            match fetch_code_status(cfg, &state.token) {
                Ok(_) => return Ok(()),
                Err(e) if is_auth_error(&e) => {
                    *session_state = None;
                    clear_cache();
                }
                Err(e) => return Err(format!("网络/服务错误：{}", e)),
            }
        }
    }

    if let Some(cache) = load_cached_session() {
        if cache.expires_at > Utc::now() {
            match fetch_code_status(cfg, &cache.token) {
                Ok(_) => {
                    *session_state = Some(cache);
                    return Ok(());
                }
                Err(e) if is_auth_error(&e) => {
                    clear_cache();
                }
                Err(e) => return Err(format!("网络/服务错误：{}", e)),
            }
        } else {
            clear_cache();
        }
    }

    let challenge = request_challenge(cfg, fingerprint)?;
    let nonce = B64_STANDARD
        .decode(challenge.nonce.as_bytes())
        .map_err(|e| format!("nonce 解码失败: {}", e))?;
    let mut message = Vec::with_capacity(SIGNING_PREFIX.len() + nonce.len());
    message.extend_from_slice(SIGNING_PREFIX);
    message.extend_from_slice(&nonce);
    let signature = signing_key.sign(&message);
    let signature_b64 = B64_STANDARD.encode(signature.to_bytes());
    let verify = verify_challenge(cfg, &challenge.challenge_id, fingerprint, &signature_b64)?;
    if verify.fingerprint.trim() != fingerprint {
        return Err("服务器返回的指纹与本地不一致".into());
    }
    let expires_at = DateTime::parse_from_rfc3339(&verify.expires_at)
        .map_err(|e| format!("解析会话过期时间失败: {}", e))?
        .with_timezone(&Utc);
    let state = SessionState {
        token: verify.token.clone(),
        expires_at,
    };
    let cache = SessionCache {
        token: verify.token,
        expires_at: verify.expires_at,
    };
    let _ = save_cache(&cache);
    *session_state = Some(state);
    Ok(())
}

/// 主程序运行循环
///
/// 运行 TUI 应用程序的主循环，处理用户输入、状态更新和界面渲染
///
/// # Arguments
///
/// * `terminal` - 终端实例
/// * `cfg` - 应用程序配置
/// * `signing_key` - 管理员签名私钥
/// * `fingerprint` - 管理员公钥指纹
///
/// # Returns
///
/// 运行结果
fn run(
    mut terminal: ratatui::Terminal<CrosstermBackend<Stdout>>,
    cfg: Config,
    signing_key: SigningKey,
    fingerprint: String,
) -> TermResult<()> {
    let mut session_state = load_cached_session();
    let mut signing_key = signing_key; // 允许在“重置密钥”后更新本地私钥
    let mut fingerprint = fingerprint; // 同步更新指纹并驱动 UI 标记

    let mut app = AppState {
        message: "准备就绪。按 R 测试连接；G/M 生成凭证".into(),
        cfg,
        connected: false,
        connect_error: None,
        last: None,
        last_code: None,
        last_code_max_uses: None,
        last_code_remaining: None,
        last_code_uses: None,
        last_code_disabled: None,
        last_magic: None,
        last_magic_remaining: None,
        last_magic_disabled: None,
        input_mode: InputMode::None,
        select_copy: false,
        fingerprint: fingerprint.clone(),
        session_expires_at: session_state.as_ref().map(|s| s.expires_at),
        screen: ui::Screen::Home,
        menu_index: 0,
        system_tab: 0,
        tokens: Vec::new(),
        selected_token: 0,
        token_offset: 0,
        admin_keys: Vec::new(),
        selected_key: 0,
        key_offset: 0,
        toast: None,
        toast_deadline: None,
    };

    match connectivity_check(&app.cfg) {
        Ok(()) => {
            app.connected = true;
            app.connect_error = None;
        }
        Err(e) => {
            app.connected = false;
            app.connect_error = Some(e);
        }
    }

    let tick_rate = Duration::from_millis(33);
    let mut last_status_poll = Instant::now();
    loop {
        if let Some(deadline) = app.toast_deadline {
            if Instant::now() >= deadline {
                app.toast = None;
                app.toast_deadline = None;
            }
        }
        if last_status_poll.elapsed() >= STATUS_POLL_INTERVAL {
            let code_should_poll = if let Some((_, exp)) = &app.last_code {
                credential_valid(exp)
                    && !app.last_code_disabled.unwrap_or(false)
                    && app.last_code_remaining.unwrap_or(1) > 0
            } else { false };
            let magic_should_poll = if let Some((_, exp)) = &app.last_magic {
                credential_valid(exp)
                    && !app.last_magic_disabled.unwrap_or(false)
                    && app.last_magic_remaining.unwrap_or(1) > 0
            } else { false };
            if code_should_poll || magic_should_poll {
                if let Some(state) = &session_state {
                    match fetch_code_status(&app.cfg, &state.token) {
                        Ok(resp) => {
                            if let Some(info) = resp.info {
                                let now_disabled = info.disabled || info.remaining_uses == 0;
                                if app.last_code.is_some() {
                                    let was_code_disabled = app.last_code_disabled.unwrap_or(false);
                                    app.last_code_remaining = Some(info.remaining_uses);
                                    app.last_code_uses = Some(info.uses);
                                    app.last_code_disabled = Some(now_disabled);
                                    if now_disabled && !was_code_disabled { app.message = "Code 已失效或次数耗尽，请重新生成".into(); }
                                }
                                if app.last_magic.is_some() {
                                    let was_magic_disabled = app.last_magic_disabled.unwrap_or(false);
                                    app.last_magic_remaining = Some(info.remaining_uses);
                                    app.last_magic_disabled = Some(now_disabled);
                                    if now_disabled && !was_magic_disabled { app.message = "Magic URL 已失效或次数耗尽，请重新生成".into(); }
                                }
                            } else {
                                if app.last_code.is_some() && !app.last_code_disabled.unwrap_or(false) {
                                    app.last_code_disabled = Some(true);
                                    app.last_code_remaining = Some(0);
                                    app.message = "Code 已失效或次数耗尽，请重新生成".into();
                                }
                                if app.last_magic.is_some() && !app.last_magic_disabled.unwrap_or(false) {
                                    app.last_magic_disabled = Some(true);
                                    app.last_magic_remaining = Some(0);
                                    if app.last_code.is_none() { app.message = "Magic URL 已失效或次数耗尽，请重新生成".into(); }
                                }
                            }
                        }
                        Err(err) => {
                            app.message = format!("查询 Code 状态失败：{}", err);
                        }
                    }
                }
            }
            last_status_poll = Instant::now();
        }

        terminal.draw(|f| render(f, &app))?;
        if event::poll(tick_rate)? {
            if let Event::Key(key) = event::read()? {
                match (app.input_mode, key.code) {
                    (InputMode::None, KeyCode::Char('h')) | (InputMode::None, KeyCode::Char('H'))
                        if matches!(app.screen, ui::Screen::Dashboard | ui::Screen::System) =>
                    {
                        app.screen = ui::Screen::Home;
                        app.message = "返回首页".into();
                    }
                    (InputMode::None, KeyCode::Up) if app.screen == ui::Screen::Home => {
                        if app.menu_index > 0 { app.menu_index -= 1; }
                    }
                    (InputMode::None, KeyCode::Down) if app.screen == ui::Screen::Home => {
                        if app.menu_index < 3 { app.menu_index += 1; }
                    }
                    (InputMode::None, KeyCode::Enter) if app.screen == ui::Screen::Home => {
                        match app.menu_index {
                            0 => { app.screen = ui::Screen::Dashboard; app.message = "进入登录凭证管理".into(); }
                            1 => {
                                // 重置 TUI 验证密钥：生成新私钥，上传公钥，并弹窗提示
                                if let Some(state) = session_state.clone() {
                                    // 生成新密钥
                                    let mut seed = [0u8; 32];
                                    if let Err(e) = getrandom::getrandom(&mut seed) {
                                        set_toast(&mut app, format!("生成随机种子失败：{}", e), 3);
                                        continue;
                                    }
                                    let new_key = ed25519_dalek::SigningKey::from_bytes(&seed);
                                    let pub_b64 = B64_STANDARD.encode(new_key.verifying_key().to_bytes());
                                    let comment = Some("rotated-by-tui");
                                    match add_admin_key(&app.cfg, &state.token, &pub_b64, comment) {
                                        Ok(_out) => {
                                            // 保存到 config
                                            let priv_b64 = B64_STANDARD.encode(new_key.to_bytes());
                                            app.cfg.private_key_b64 = Some(priv_b64);
                                            let _ = save_config(&app.cfg);
                                            // 切换内存中的签名密钥与指纹，并立即建立新会话
                                            signing_key = new_key;
                                            let new_fp = cfg_fingerprint(&app.cfg).unwrap_or_default();
                                            fingerprint = new_fp.clone();
                                            app.fingerprint = new_fp;
                                            if let Err(e) = ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state) {
                                                set_toast(&mut app, format!("密钥已切换，但建立新会话失败：{}", e), 5);
                                            } else if let Some(s) = &session_state {
                                                app.session_expires_at = Some(s.expires_at);
                                                set_toast(&mut app, "新公钥上传成功，本地私钥和会话已更新", 3);
                                            }
                                        }
                                        Err(e) => set_toast(&mut app, format!("上传新密钥失败：{}", e), 3),
                                    }
                                } else {
                                    set_toast(&mut app, "会话未初始化，无法重置", 3);
                                }
                            }
                            2 => {
                                // 进入系统管理：加载令牌与密钥列表
                                // 进入系统管理前，确保管理员会话有效
                                if let Err(e) = ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state) {
                                    set_toast(&mut app, format!("进入系统管理失败：{}", e), 4);
                                } else if let Some(state) = &session_state {
                                    if let Ok(ts) = list_tokens(&app.cfg, &state.token) { app.tokens = ts; app.selected_token = 0; }
                                    if let Ok(mut keys) = list_admin_keys(&app.cfg, &state.token) { sort_keys_desc(&mut keys); app.admin_keys = keys; app.selected_key = 0; }
                                    app.system_tab = 0;
                                    app.screen = ui::Screen::System;
                                }
                            }
                            3 => { app.input_mode = InputMode::ConfirmQuit; }
                            _ => {}
                        }
                    }
                    (
                        InputMode::Params {
                            mut selected,
                            mut draft_ttl,
                            mut draft_uses,
                            mut draft_len,
                        },
                        k,
                    ) => match k {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::None;
                        }
                        KeyCode::Up => {
                            if selected > 0 {
                                selected -= 1;
                            }
                            app.input_mode = InputMode::Params {
                                selected,
                                draft_ttl,
                                draft_uses,
                                draft_len,
                            };
                        }
                        KeyCode::Down => {
                            if selected < 2 {
                                selected += 1;
                            }
                            app.input_mode = InputMode::Params {
                                selected,
                                draft_ttl,
                                draft_uses,
                                draft_len,
                            };
                        }
                        KeyCode::Left => {
                            match selected {
                                0 => {
                                    if draft_ttl > 1 {
                                        draft_ttl = draft_ttl.saturating_sub(1).max(1);
                                    }
                                }
                                1 => {
                                    if draft_uses > 1 {
                                        draft_uses -= 1;
                                    }
                                }
                                2 => {
                                    if draft_len > 25 {
                                        draft_len -= 1;
                                    }
                                }
                                _ => {}
                            }
                            app.input_mode = InputMode::Params {
                                selected,
                                draft_ttl,
                                draft_uses,
                                draft_len,
                            };
                        }
                        KeyCode::Right => {
                            match selected {
                                0 => {
                                    if draft_ttl < 86400 {
                                        draft_ttl = (draft_ttl + 1).min(86400);
                                    }
                                }
                                1 => {
                                    if draft_uses < 1000 {
                                        draft_uses += 1;
                                    }
                                }
                                2 => {
                                    if draft_len < 64 {
                                        draft_len += 1;
                                    }
                                }
                                _ => {}
                            }
                            app.input_mode = InputMode::Params {
                                selected,
                                draft_ttl,
                                draft_uses,
                                draft_len,
                            };
                        }
                        KeyCode::Enter => {
                            app.cfg.ttl_secs = draft_ttl;
                            app.cfg.max_uses = draft_uses;
                            app.cfg.length = draft_len;
                            match save_config(&app.cfg) {
                                Ok(()) => app.message = "参数已更新并保存".into(),
                                Err(e) => app.message = format!("保存失败：{}", e),
                            }
                            app.input_mode = InputMode::None;
                        }
                        _ => {}
                    },
                    (InputMode::ConfirmQuit, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) => {
                        return Ok(());
                    }
                    (InputMode::ConfirmQuit, KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc) => {
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::ConfirmQuit, _) => {}
                    (InputMode::TokenDetails { .. }, KeyCode::Esc | KeyCode::Enter) => {
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::CopyChoice, KeyCode::Esc) => {
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::CopyChoice, KeyCode::Char('1')) => {
                        if let Some((code, exp)) = &app.last_code {
                            let usable = app.last_code_remaining.unwrap_or(1) > 0
                                && !app.last_code_disabled.unwrap_or(false);
                            if credential_valid(exp) && usable {
                                match copy_to_clipboard(code) {
                                    Ok(()) => app.message = "已复制 Code".into(),
                                    Err(e) => app.message = format!("复制失败：{}", e),
                                }
                            } else {
                                app.message = "Code 已失效或次数耗尽，请重新生成".into();
                            }
                        } else {
                            app.message = "暂无可复制的 Code".into();
                        }
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::CopyChoice, KeyCode::Char('2')) => {
                        if let Some((url, exp)) = &app.last_magic {
                            if credential_valid(exp) {
                                match copy_to_clipboard(url) {
                                    Ok(()) => app.message = "已复制 Magic URL".into(),
                                    Err(e) => app.message = format!("复制失败：{}", e),
                                }
                            } else {
                                app.message = "Magic URL 已过期，请重新生成".into();
                            }
                        } else {
                            app.message = "暂无可复制的 Magic URL".into();
                        }
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::CopyChoice, KeyCode::Char('3')) => {
                        let mut buf = String::new();
                        if let Some((code, exp)) = &app.last_code {
                            let usable = app.last_code_remaining.unwrap_or(1) > 0
                                && !app.last_code_disabled.unwrap_or(false);
                            if credential_valid(exp) && usable {
                                buf.push_str(code);
                            }
                        }
                        if let Some((url, exp)) = &app.last_magic {
                            if credential_valid(exp) {
                                if !buf.is_empty() {
                                    buf.push('\n');
                                }
                                buf.push_str(url);
                            }
                        }
                        if buf.is_empty() {
                            app.message = "暂无有效凭证可复制，请重新生成".into();
                        } else {
                            match copy_to_clipboard(&buf) {
                                Ok(()) => app.message = "已复制 Code + Magic URL".into(),
                                Err(e) => app.message = format!("复制失败：{}", e),
                            }
                        }
                        app.input_mode = InputMode::None;
                    }
                    (_, KeyCode::Char('q') | KeyCode::Char('Q')) => {
                        app.input_mode = InputMode::ConfirmQuit;
                    }
                    (_, KeyCode::Char('g') | KeyCode::Char('G')) if app.screen == ui::Screen::Dashboard => {
                        if let Err(e) =
                            ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state)
                        {
                            app.message = format!("生成失败：{}", e);
                            continue;
                        }
                        if let Some(state) = &session_state {
                            app.session_expires_at = Some(state.expires_at);
                        }
                        match session_state
                            .as_ref()
                            .ok_or_else(|| "会话尚未初始化".to_string())
                            .and_then(|state| {
                                create_code(
                                    &app.cfg,
                                    &state.token,
                                    app.cfg.ttl_secs,
                                    app.cfg.max_uses,
                                    app.cfg.length,
                                    false,
                                )
                            }) {
                            Ok(r) => {
                                app.last_code = Some((r.code.clone(), r.expires_at.clone()));
                                app.last_code_max_uses = Some(r.max_uses);
                                app.last_code_remaining = Some(r.remaining_uses);
                                app.last_code_uses = Some(r.uses);
                                app.last_code_disabled = Some(false);
                                app.last = Some(Credential::Code {
                                    code: r.code.clone(),
                                    expires_at: r.expires_at.clone(),
                                });
                                app.message = "已生成 Code\n按 Y 复制，S 保存。".into();
                            }
                            Err(e) => {
                                if is_auth_error(&e) {
                                    session_state = None;
                                    clear_cache();
                                    if let Err(err) = ensure_session(
                                        &app.cfg,
                                        &signing_key,
                                        &fingerprint,
                                        &mut session_state,
                                    ) {
                                        app.message = format!("生成失败：{}", err);
                                        continue;
                                    }
                                    if let Some(state) = &session_state {
                                        app.session_expires_at = Some(state.expires_at);
                                    }
                                    match session_state
                                        .as_ref()
                                        .ok_or_else(|| "会话尚未初始化".to_string())
                                        .and_then(|state| {
                                            create_code(
                                                &app.cfg,
                                                &state.token,
                                                app.cfg.ttl_secs,
                                                app.cfg.max_uses,
                                                app.cfg.length,
                                                false,
                                            )
                                        }) {
                                        Ok(r) => {
                                            app.last_code =
                                                Some((r.code.clone(), r.expires_at.clone()));
                                            app.last_code_max_uses = Some(r.max_uses);
                                            app.last_code_remaining = Some(r.remaining_uses);
                                            app.last_code_uses = Some(r.uses);
                                            app.last_code_disabled = Some(false);
                                            app.last = Some(Credential::Code {
                                                code: r.code.clone(),
                                                expires_at: r.expires_at.clone(),
                                            });
                                            app.message = "已生成 Code\n按 Y 复制，S 保存。".into();
                                        }
                                        Err(err) => app.message = format!("生成失败：{}", err),
                                    }
                                } else {
                                    app.message = format!("生成失败：{}", e);
                                }
                            }
                        }
                    }
                    (_, KeyCode::Char('m') | KeyCode::Char('M')) if app.screen == ui::Screen::Dashboard => {
                        if let Err(e) =
                            ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state)
                        {
                            app.message = format!("生成失败：{}", e);
                            continue;
                        }
                        if let Some(state) = &session_state {
                            app.session_expires_at = Some(state.expires_at);
                        }
                        match session_state
                            .as_ref()
                            .ok_or_else(|| "会话尚未初始化".to_string())
                            .and_then(|state| {
                                create_code(
                                    &app.cfg,
                                    &state.token,
                                    app.cfg.ttl_secs,
                                    app.cfg.max_uses,
                                    app.cfg.length,
                                    true,
                                )
                            }) {
                            Ok(r) => {
                                let raw = r
                                    .login_url
                                    .unwrap_or_else(|| format!("/#/auth/magic?code={}", r.code));
                                let url = resolve_magic_url(&app.cfg, &raw);
                                app.last_magic = Some((url.clone(), r.expires_at.clone()));
                                app.last_magic_remaining = Some(r.remaining_uses);
                                app.last_magic_disabled = Some(false);
                                // 生成 Magic URL 时清空上一次 Code 显示，避免误解为同时生成了 Code
                                app.last_code = None;
                                app.last_code_remaining = None;
                                app.last_code_uses = None;
                                app.last_code_disabled = None;
                                app.last = Some(Credential::Magic {
                                    url: url.clone(),
                                    expires_at: r.expires_at.clone(),
                                });
                                app.message = "已生成 Magic URL\n按 Y 复制，S 保存。".into();
                            }
                            Err(e) => {
                                if is_auth_error(&e) {
                                    session_state = None;
                                    clear_cache();
                                    if let Err(err) = ensure_session(
                                        &app.cfg,
                                        &signing_key,
                                        &fingerprint,
                                        &mut session_state,
                                    ) {
                                        app.message = format!("生成失败：{}", err);
                                        continue;
                                    }
                                    if let Some(state) = &session_state {
                                        app.session_expires_at = Some(state.expires_at);
                                    }
                                    match session_state
                                        .as_ref()
                                        .ok_or_else(|| "会话尚未初始化".to_string())
                                        .and_then(|state| {
                                            create_code(
                                                &app.cfg,
                                                &state.token,
                                                app.cfg.ttl_secs,
                                                app.cfg.max_uses,
                                                app.cfg.length,
                                                true,
                                            )
                                        }) {
                                        Ok(r) => {
                                            let raw = r.login_url.unwrap_or_else(|| {
                                                format!("/#/auth/magic?code={}", r.code)
                                            });
                                            let url = resolve_magic_url(&app.cfg, &raw);
                                            app.last_magic =
                                                Some((url.clone(), r.expires_at.clone()));
                                            app.last_magic_remaining = Some(r.remaining_uses);
                                            app.last_magic_disabled = Some(false);
                                            // 生成 Magic URL 时清空上一次 Code 显示，避免误解为同时生成了 Code
                                            app.last_code = None;
                                            app.last_code_remaining = None;
                                            app.last_code_uses = None;
                                            app.last_code_disabled = None;
                                            app.last = Some(Credential::Magic {
                                                url: url.clone(),
                                                expires_at: r.expires_at.clone(),
                                            });
                                            app.message =
                                                "已生成 Magic URL\n按 Y 复制，S 保存。".into();
                                        }
                                        Err(err) => app.message = format!("生成失败：{}", err),
                                    }
                                } else {
                                    app.message = format!("生成失败：{}", e);
                                }
                            }
                        }
                    }
                    (_, KeyCode::Char('r') | KeyCode::Char('R')) if app.screen != ui::Screen::System => {
                        // R：同时做联网检查与会话验证（挑战-应答）
                        match ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state) {
                            Ok(()) => {
                                app.connected = true;
                                app.connect_error = None;
                                if let Some(state) = &session_state { app.session_expires_at = Some(state.expires_at); }
                                app.message = "连接正常，管理员会话有效".into();
                            }
                            Err(e) => {
                                app.connected = false;
                                app.connect_error = Some(e.clone());
                                app.message = format!("连接/认证失败：{}", e);
                            }
                        }
                    }
                    // System 管理快捷键
                    (_, KeyCode::Tab) if app.screen == ui::Screen::System => {
                        app.system_tab = if app.system_tab == 0 { 1 } else { 0 };
                    }
                    (_, KeyCode::Up) if app.screen == ui::Screen::System => {
                        if app.system_tab == 0 {
                            if app.selected_token > 0 { app.selected_token -= 1; }
                            if app.selected_token < app.token_offset { app.token_offset = app.selected_token; }
                        } else if app.selected_key > 0 {
                            app.selected_key -= 1;
                            if app.selected_key < app.key_offset { app.key_offset = app.selected_key; }
                        }
                    }
                    (_, KeyCode::Down) if app.screen == ui::Screen::System => {
                        if app.system_tab == 0 {
                            if app.selected_token + 1 < app.tokens.len() { app.selected_token += 1; }
                            if app.selected_token >= app.token_offset + SYSTEM_VISIBLE_ROWS {
                                app.token_offset = app.selected_token.saturating_sub(SYSTEM_VISIBLE_ROWS - 1);
                            }
                        } else if app.selected_key + 1 < app.admin_keys.len() {
                            app.selected_key += 1;
                            if app.selected_key >= app.key_offset + SYSTEM_VISIBLE_ROWS {
                                app.key_offset = app.selected_key.saturating_sub(SYSTEM_VISIBLE_ROWS - 1);
                            }
                        }
                    }
                    // Enter 查看令牌详情（左侧）
                    (_, KeyCode::Enter) if app.screen == ui::Screen::System && app.system_tab == 0 => {
                        if app.selected_token < app.tokens.len() {
                            app.input_mode = InputMode::TokenDetails { index: app.selected_token };
                        }
                    }
                    // Delete 删除密钥（右侧）- 二次确认
                    (_, KeyCode::Delete | KeyCode::Backspace) if app.screen == ui::Screen::System && app.system_tab == 1 => {
                        if app.selected_key < app.admin_keys.len() {
                            app.input_mode = InputMode::ConfirmDeleteKey { index: app.selected_key };
                        }
                    }
                    // 确认对话框：删除密钥
                    (InputMode::ConfirmDeleteKey { index }, KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) => {
                        // 删除后可能导致基于该指纹的 TUI 会话被级联删除；删除后立刻重建会话并刷新列表
                        let sel_fp = app.admin_keys.get(index).map(|k| k.fingerprint.clone());
                        if let Some(sel_fp) = sel_fp {
                            if sel_fp == app.fingerprint {
                                set_toast(&mut app, "禁止删除当前使用中的管理员密钥", 3);
                            } else {
                                // 确保有会话或尝试重建
                                if ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state).is_err() {
                                    set_toast(&mut app, "会话无效，正在重建会话后再试", 3);
                                    let _ = ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state);
                                }
                                if let Some(state) = &session_state {
                                    match delete_admin_key(&app.cfg, &state.token, &sel_fp) {
                                        Ok(true) => set_toast(&mut app, "密钥已删除", 3),
                                        Ok(false) => set_toast(&mut app, "密钥删除失败", 3),
                                        Err(e) => set_toast(&mut app, format!("删除失败：{}", e), 3),
                                    }
                                }
                                // 删除后，无论结果如何，重建/验证会话并刷新两侧列表
                                if let Err(e) = ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state) {
                                    set_toast(&mut app, format!("会话已失效：{}", e), 4);
                                }
                                if let Some(state) = &session_state {
                                    if let Ok(ts) = list_tokens(&app.cfg, &state.token) { app.tokens = ts; }
                                    if let Ok(mut keys) = list_admin_keys(&app.cfg, &state.token) { sort_keys_desc(&mut keys); app.admin_keys = keys; }
                                }
                            }
                        }
                        app.input_mode = InputMode::None;
                    }
                    (InputMode::ConfirmDeleteKey { .. }, KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc) => {
                        app.input_mode = InputMode::None;
                    }
                    (_, KeyCode::Char('r') | KeyCode::Char('R')) if app.screen == ui::Screen::System => {
                        if let Err(e) = ensure_session(&app.cfg, &signing_key, &fingerprint, &mut session_state) {
                            set_toast(&mut app, format!("刷新失败：{}", e), 3);
                        } else if let Some(state) = &session_state {
                            if let Ok(ts) = list_tokens(&app.cfg, &state.token) { app.tokens = ts; }
                            if let Ok(mut keys) = list_admin_keys(&app.cfg, &state.token) { sort_keys_desc(&mut keys); app.admin_keys = keys; }
                            set_toast(&mut app, "已刷新", 2);
                        }
                    }
                    (_, KeyCode::Char('y') | KeyCode::Char('Y')) => {
                        let code_valid = app
                            .last_code
                            .as_ref()
                            .map(|(_, exp)| credential_valid(exp))
                            .unwrap_or(false)
                            && app.last_code_remaining.unwrap_or(1) > 0
                            && !app.last_code_disabled.unwrap_or(false);
                        let magic_valid = app
                            .last_magic
                            .as_ref()
                            .map(|(_, exp)| credential_valid(exp))
                            .unwrap_or(false);

                        match (code_valid, magic_valid) {
                            (true, true) => {
                                app.input_mode = InputMode::CopyChoice;
                            }
                            (true, false) => {
                                if let Some((code, _)) = &app.last_code {
                                    match copy_to_clipboard(code) {
                                        Ok(()) => app.message = "已复制 Code 到剪贴板".into(),
                                        Err(e) => app.message = format!("复制失败：{}", e),
                                    }
                                } else {
                                    app.message = "没有可复制的 Code".into();
                                }
                            }
                            (false, true) => {
                                if let Some((url, _)) = &app.last_magic {
                                    match copy_to_clipboard(url) {
                                        Ok(()) => app.message = "已复制 Magic URL 到剪贴板".into(),
                                        Err(e) => app.message = format!("复制失败：{}", e),
                                    }
                                } else {
                                    app.message = "没有可复制的 Magic URL".into();
                                }
                            }
                            (false, false) => {
                                app.message = "Code 和 Magic URL 均已过期，请重新生成。".into();
                            }
                        }
                    }
                    (_, KeyCode::Char('s') | KeyCode::Char('S')) => {
                        if let Some(last) = &app.last {
                            match save_last_to_file(last) {
                                Ok(p) => app.message = format!("已保存到 {}", p),
                                Err(e) => app.message = format!("保存失败：{}", e),
                            }
                        } else {
                            app.message = "无可保存的凭证".into();
                        }
                    }
                    (_, KeyCode::Char('c') | KeyCode::Char('C')) => {
                        app.input_mode = InputMode::Params {
                            selected: 0,
                            draft_ttl: app.cfg.ttl_secs,
                            draft_uses: app.cfg.max_uses,
                            draft_len: app.cfg.length,
                        };
                    }
                    (_, KeyCode::Char('v') | KeyCode::Char('V')) => {
                        app.select_copy = !app.select_copy;
                        if app.select_copy {
                            let _ = crossterm::execute!(
                                terminal.backend_mut(),
                                crossterm::event::DisableMouseCapture
                            );
                            app.message = "已开启：允许鼠标选择复制（再次按 V 恢复）".into();
                        } else {
                            let _ = crossterm::execute!(
                                terminal.backend_mut(),
                                crossterm::event::EnableMouseCapture
                            );
                            app.message = "已关闭：允许选择复制".into();
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// 主函数
///
/// 初始化配置、加载管理员密钥、初始化终端并启动 TUI 应用程序
///
/// # Returns
///
/// 程序运行结果
fn main() -> TermResult<()> {
    let cfg = ensure_config_interactive();
    let signing_key =
        load_signing_key(&cfg).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let fingerprint = cfg_fingerprint(&cfg).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    println!("正在验证管理员会话...");
    let mut preflight_state: Option<SessionState> = None;
    match ensure_session(&cfg, &signing_key, &fingerprint, &mut preflight_state) {
        Ok(()) => {
            if let Some(state) = preflight_state {
                println!(
                    "管理员认证成功，会话有效至 {}",
                    state.expires_at.to_rfc3339()
                );
            }
        }
        Err(err) => {
            eprintln!("管理员会话验证失败：{}\n将继续进入界面，您可按 R 测试连接或检查密钥配置。", err);
        }
    }

    let terminal = init_terminal()?;
    let result = run(terminal, cfg, signing_key, fingerprint);
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let terminal = ratatui::Terminal::new(backend)?;
    let _ = restore_terminal(terminal);
    result
}

/// 设置 Toast 通知
///
/// 显示一个临时的 Toast 通知消息，在指定时间后自动消失
///
/// # Arguments
///
/// * `app` - 应用程序状态的可变引用
/// * `msg` - 要显示的消息
/// * `secs` - 显示时间（秒）
fn set_toast(app: &mut AppState, msg: impl Into<String>, secs: u64) {
    app.toast = Some(msg.into());
    app.toast_deadline = Some(Instant::now() + Duration::from_secs(secs));
}

/// 按创建时间降序排列管理员密钥
///
/// 将管理员密钥列表按创建时间从新到旧排序
///
/// # Arguments
///
/// * `keys` - 要排序的管理员密钥列表
fn sort_keys_desc(keys: &mut Vec<crate::api::AdminKeyOut>) {
    keys.sort_by(|a, b| b.created_at.cmp(&a.created_at));
}
