//! Real-time EEG chart viewer for Emotiv headsets.
//!
//! # Usage
//!
//! ```bash
//! # Real headset (requires EMOTIV Launcher + API credentials)
//! cargo run --bin emotiv-tui
//!
//! # Direct raw BLE mode (no Cortex API)
//! cargo run --bin emotiv-tui --features raw -- --raw
//! cargo run --bin emotiv-tui --features raw -- --raw --raw-index 0
//! cargo run --bin emotiv-tui --features raw -- --raw --raw-device EPOC_1234
//!
//! # Simulated signal (no hardware or credentials needed)
//! cargo run --bin emotiv-tui --features simulate -- --simulate
//! ```
//!
//! # API credentials
//!
//! Create a Cortex App at <https://www.emotiv.com/my-account/cortex-apps/>
//! to get a Client ID and Client Secret, then export them:
//!
//! ```bash
//! export EMOTIV_CLIENT_ID="your_client_id"
//! export EMOTIV_CLIENT_SECRET="your_client_secret"
//! ```
//!
//! On the first run the EMOTIV Launcher will ask you to approve the app.
//!
//! # Keys (all modes)
//!
//! | Key | Action |
//! |-----|--------|
//! | `Tab` | Cycle views (EEG → Metrics → BandPower [→ SimControl]) |
//! | `+`/`=` | Zoom out (increase µV scale) |
//! | `-` | Zoom in |
//! | `a` | Auto-scale Y axis |
//! | `v` | Toggle smooth overlay |
//! | `p` / `r` | Pause / resume |
//! | `c` | Clear buffers |
//! | `q` / `Esc` | Quit |
//!
//! # Keys (simulate mode — `--features simulate`)
//!
//! | Key | Action |
//! |-----|--------|
//! | `1`–`5` | Brain state: 1=Relaxed 2=Focused 3=Excited 4=Drowsy 5=Meditative |
//! | `b` | Eye blink artifact (200 ms frontal spike) |
//! | `j` | Jaw clench artifact (500 ms temporal EMG burst) |
//! | `m` | Cycle mental command (neutral → push → pull → lift → drop) |
//! | `f` | Cycle facial expression (neutral → smile → surprise → frown → clench) |
//! | `n`/`N` | Noise level ↓/↑ |
//! | `g`/`G` | Signal gain ↓/↑ |

use std::collections::VecDeque;
#[cfg(feature = "simulate")]
use std::f64::consts::PI;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
    Frame, Terminal,
};

use emotiv::types::*;
#[cfg(feature = "raw")]
use emotiv::raw;

fn normalize_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.iter()
        .position(|a| a == key)
        .and_then(|idx| args.get(idx + 1).cloned())
}

#[cfg(feature = "raw")]
fn select_best_device(devices: &[raw::DeviceInfo]) -> Option<&raw::DeviceInfo> {
    let likely: Vec<&raw::DeviceInfo> = devices.iter().filter(|d| is_likely_emotiv(d)).collect();
    if !likely.is_empty() {
        likely.into_iter().max_by_key(|d| device_score(d))
    } else {
        devices.iter().max_by_key(|d| device_score(d))
    }
}

#[cfg(feature = "raw")]
fn device_score(d: &raw::DeviceInfo) -> i32 {
    let mut score = 0;
    let name = d.name.to_ascii_lowercase();
    if name != "(unknown)" {
        score += 20;
    }
    if name.contains("emotiv")
        || name.contains("epoc")
        || name.contains("insight")
        || name.contains("flex")
        || name.contains("mn8")
        || name.contains("xtrodes")
    {
        score += 50;
    }
    if d.ble_mac.as_deref().is_some_and(|m| !is_zero_mac(m)) {
        score += 15;
    }
    if !normalize_id(&d.ble_id).is_empty() {
        score += 10;
    }
    if d.is_connected {
        score += 10;
    }
    score
}

#[cfg(feature = "raw")]
fn is_likely_emotiv(d: &raw::DeviceInfo) -> bool {
    let name = d.name.to_ascii_lowercase();
    if name.contains("emotiv")
        || name.contains("epoc")
        || name.contains("insight")
        || name.contains("flex")
        || name.contains("mn8")
        || name.contains("xtrodes")
    {
        return true;
    }

    if name == "(unknown)" || name.starts_with("gb-") || name.starts_with("ble") {
        return false;
    }

    d.is_connected
}

#[cfg(feature = "raw")]
fn is_zero_mac(mac: &str) -> bool {
    let only_hex: String = mac
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase();
    !only_hex.is_empty() && only_hex.chars().all(|c| c == '0')
}

// ── Constants ─────────────────────────────────────────────────────────────────

const WINDOW_SECS: f64 = 2.0;
const EEG_HZ: f64 = 128.0;
const BUF_SIZE: usize = (WINDOW_SECS * EEG_HZ) as usize;
const MAX_CHANNELS: usize = 14;
const Y_SCALES: &[f64] = &[10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0];
const DEFAULT_SCALE: usize = 5;
const SMOOTH_WINDOW: usize = 9;
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const COLORS: [Color; 14] = [
    Color::Cyan, Color::Yellow, Color::Green, Color::Magenta,
    Color::LightRed, Color::LightBlue, Color::LightGreen, Color::LightCyan,
    Color::Red, Color::Blue, Color::White, Color::LightYellow,
    Color::LightMagenta, Color::Gray,
];

const DIM_COLORS: [Color; 14] = [
    Color::Rgb(0, 90, 110), Color::Rgb(110, 90, 0), Color::Rgb(0, 110, 0), Color::Rgb(110, 0, 110),
    Color::Rgb(110, 40, 40), Color::Rgb(40, 40, 110), Color::Rgb(40, 110, 40), Color::Rgb(40, 110, 110),
    Color::Rgb(110, 0, 0), Color::Rgb(0, 0, 110), Color::Rgb(80, 80, 80), Color::Rgb(110, 110, 40),
    Color::Rgb(110, 40, 110), Color::Rgb(60, 60, 60),
];

// ══════════════════════════════════════════════════════════════════════════════
//  Simulation types — only compiled with `--features simulate`
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "simulate")]
mod sim {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BrainState { Relaxed, Focused, Excited, Drowsy, Meditative }

    impl BrainState {
        pub fn name(&self) -> &'static str {
            match self { Self::Relaxed=>"Relaxed", Self::Focused=>"Focused", Self::Excited=>"Excited", Self::Drowsy=>"Drowsy", Self::Meditative=>"Meditative" }
        }
        pub fn color(&self) -> Color {
            match self { Self::Relaxed=>Color::Green, Self::Focused=>Color::Yellow, Self::Excited=>Color::Red, Self::Drowsy=>Color::Blue, Self::Meditative=>Color::Magenta }
        }
        /// (theta, alpha, beta, gamma) amplitudes in µV
        pub fn band_amplitudes(&self) -> (f64,f64,f64,f64) {
            match self {
                Self::Relaxed    => (8.0, 25.0, 5.0, 2.0),
                Self::Focused    => (5.0, 8.0, 20.0, 8.0),
                Self::Excited    => (5.0, 6.0, 25.0, 15.0),
                Self::Drowsy     => (20.0,12.0, 3.0, 1.0),
                Self::Meditative => (15.0,22.0, 4.0, 2.0),
            }
        }
        /// (eng, exc, lex, str, rel, int, foc)
        pub fn metrics(&self) -> [f64;7] {
            match self {
                Self::Relaxed    => [0.3,0.2,0.15,0.1,0.9,0.4,0.3],
                Self::Focused    => [0.8,0.5,0.3,0.4,0.3,0.7,0.9],
                Self::Excited    => [0.9,0.9,0.8,0.6,0.2,0.8,0.7],
                Self::Drowsy     => [0.2,0.1,0.05,0.1,0.7,0.2,0.1],
                Self::Meditative => [0.4,0.3,0.2,0.05,0.95,0.5,0.6],
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ArtifactKind { None, Blink, JawClench }

    pub const MC_ACTIONS: &[&str] = &["neutral","push","pull","lift","drop"];
    pub const FE_ACTIONS: &[&str] = &["neutral","smile","surprise","frown","clench"];

    pub struct SimState {
        pub brain_state: BrainState,
        pub cur_theta: f64, pub cur_alpha: f64, pub cur_beta: f64, pub cur_gamma: f64,
        pub cur_metrics: [f64;7],
        pub noise_level: f64,
        pub gain: f64,
        pub artifact: ArtifactKind,
        pub artifact_end: f64,
        pub mc_idx: usize, pub mc_power: f64,
        pub fe_idx: usize, pub fe_power: f64,
        pub battery: f64,
    }

    impl SimState {
        pub fn new() -> Self {
            let (th,al,be,ga) = BrainState::Relaxed.band_amplitudes();
            let m = BrainState::Relaxed.metrics();
            Self {
                brain_state:BrainState::Relaxed,
                cur_theta:th, cur_alpha:al, cur_beta:be, cur_gamma:ga,
                cur_metrics:m, noise_level:0.5, gain:1.0,
                artifact:ArtifactKind::None, artifact_end:0.0,
                mc_idx:0, mc_power:0.0, fe_idx:0, fe_power:0.0, battery:85.0,
            }
        }
        pub fn set_brain_state(&mut self, s: BrainState) { self.brain_state = s; }
        pub fn inject_artifact(&mut self, kind: ArtifactKind, t: f64) {
            self.artifact = kind;
            self.artifact_end = t + match kind { ArtifactKind::Blink=>0.2, ArtifactKind::JawClench=>0.5, ArtifactKind::None=>0.0 };
        }
        pub fn cycle_mc(&mut self) { self.mc_idx = (self.mc_idx+1)%MC_ACTIONS.len(); self.mc_power = if self.mc_idx==0{0.0}else{0.7}; }
        pub fn cycle_fe(&mut self) { self.fe_idx = (self.fe_idx+1)%FE_ACTIONS.len(); self.fe_power = if self.fe_idx==0{0.0}else{0.8}; }

        pub fn tick(&mut self) {
            let r = 0.05;
            let (th,al,be,ga) = self.brain_state.band_amplitudes();
            self.cur_theta += (th-self.cur_theta)*r;
            self.cur_alpha += (al-self.cur_alpha)*r;
            self.cur_beta  += (be-self.cur_beta)*r;
            self.cur_gamma += (ga-self.cur_gamma)*r;
            let tgt = self.brain_state.metrics();
            for i in 0..7 { self.cur_metrics[i] += (tgt[i]-self.cur_metrics[i])*r; }
            if self.mc_idx==0 { self.mc_power *= 0.95; }
        }

        pub fn gen_eeg_sample(&self, t: f64, ch: usize) -> f64 {
            let phi = ch as f64 * PI / 2.5;
            let theta = self.cur_theta * (2.0*PI*6.0*t + phi*0.9).sin();
            let alpha = self.cur_alpha * (2.0*PI*10.0*t + phi).sin();
            let beta  = self.cur_beta  * (2.0*PI*22.0*t + phui*1.7).sin();
            let gamma = self.cur_gamma * (2.0*PI*40.0*t + phi*2.3).sin();
            let nx = t*1000.7 + ch as f64*137.508;
            let noise = ((nx.sin()*9973.1).fract()-0.5)*8.0*self.noise_level;
            let mut val = (theta+alpha+beta+gamma+noise)*self.gain;
            if t < self.artifact_end {
                match self.artifact {
                    ArtifactKind::Blink => {
                        let f = if ch==0||ch==13||ch==1||ch==12{1.0}else{0.3};
                        val += 200.0*f*(2.0*PI*3.0*(t-(self.artifact_end-0.2))).sin()*self.gain;
                    }
                    ArtifactKind::JawClench => {
                        let f = if ch==4||ch==9||ch==3||ch==10{1.0}else{0.2};
                        let emg = ((t*5000.3+ch as f64*77.7).sin()*31337.0).fract()-0.5;
                        val += 150.0*f*emg*self.gain;
                    }
                    ArtifactKind::None => {}
                }
            }
            val
        }

        pub fn gen_band_power(&self, ch: usize) -> [f64;5] {
            let j = 1.0+0.1*((ch as f64*7.3).sin());
            [(self.cur_theta*0.3*j).max(0.0),(self.cur_alpha*0.3*j).max(0.0),
             (self.cur_beta*0.2*j).max(0.0),(self.cur_beta*0.1*j).max(0.0),
             (self.cur_gamma*0.15*j).max(0.0)]
        }
    }
}

// ── View mode ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode { Eeg, Metrics, BandPower, #[cfg(feature = "simulate")] SimControl }

impl ViewMode {
    fn next(self, #[allow(unused)] is_sim: bool) -> Self {
        match self {
            Self::Eeg => Self::Metrics,
            Self::Metrics => Self::BandPower,
            #[cfg(feature = "simulate")]
            Self::BandPower if is_sim => Self::SimControl,
            Self::BandPower => Self::Eeg,
            #[cfg(feature = "simulate")]
            Self::SimControl => Self::Eeg,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::Eeg => "EEG", Self::Metrics => "Metrics", Self::BandPower => "BandPow",
            #[cfg(feature = "simulate")] Self::SimControl => "SimCtrl",
        }
    }
}

// ── App state ─────────────────────────────────────────────────────────────────

struct App {
    bufs: Vec<VecDeque<f64>>,
    num_channels: usize,
    channel_labels: Vec<String>,
    view: ViewMode,
    battery: Option<f64>,
    signal: Option<f64>,
    metrics: Option<MetricsData>,
    band_power: Option<BandPowerData>,
    mc_action: Option<String>,
    mc_power: Option<f64>,
    fe_action: Option<String>,
    fe_power: Option<f64>,
    total_samples: u64,
    pkt_times: VecDeque<Instant>,
    scale_idx: usize,
    paused: bool,
    smooth: bool,
    connected: bool,
    simulated: bool,
    #[cfg(feature = "raw")]
    raw_debug: Option<RawDebugInfo>,
    #[cfg(feature = "raw")]
    raw_last_decoded_channels: usize,
    #[cfg(feature = "simulate")]
    sim: sim::SimState,
}

#[cfg(feature = "raw")]
#[derive(Clone, Default)]
struct RawDebugInfo {
    rx: u64,
    dec: u64,
    fail: u64,
    timeout: u64,
    key: String,
}

impl App {
    fn new(simulated: bool) -> Self {
        Self {
            bufs: (0..MAX_CHANNELS).map(|_| VecDeque::with_capacity(BUF_SIZE+16)).collect(),
            num_channels: if simulated{14}else{0},
            channel_labels: Vec::new(),
            view: ViewMode::Eeg,
            battery: None, signal: None,
            metrics: None, band_power: None,
            mc_action: None, mc_power: None,
            fe_action: None, fe_power: None,
            total_samples: 0,
            pkt_times: VecDeque::with_capacity(256),
            scale_idx: if simulated{2}else{DEFAULT_SCALE},
            paused: false, smooth: true,
            connected: false, simulated,
            #[cfg(feature = "raw")]
            raw_debug: None,
            #[cfg(feature = "raw")]
            raw_last_decoded_channels: 0,
            #[cfg(feature = "simulate")]
            sim: sim::SimState::new(),
        }
    }

    fn push_eeg(&mut self, samples: &[f64]) {
        if self.paused { return; }
        if self.num_channels == 0 { self.num_channels = samples.len().min(MAX_CHANNELS); }
        for (ch, &v) in samples.iter().enumerate().take(MAX_CHANNELS) {
            let buf = &mut self.bufs[ch];
            buf.push_back(v);
            while buf.len() > BUF_SIZE { buf.pop_front(); }
        }
        self.total_samples += 1;
        let now = Instant::now();
        self.pkt_times.push_back(now);
        while self.pkt_times.front().map(|t| now.duration_since(*t)>Duration::from_secs(2)).unwrap_or(false) { self.pkt_times.pop_front(); }
    }
    fn clear(&mut self) { for b in &mut self.bufs{b.clear();} self.total_samples=0; self.pkt_times.clear(); }
    fn pkt_rate(&self) -> f64 {
        let n=self.pkt_times.len(); if n<2{return 0.0;}
        let s=self.pkt_times.back().unwrap().duration_since(self.pkt_times[0]).as_secs_f64();
        if s<1e-9{0.0}else{(n as f64-1.0)/s}
    }
    fn y_range(&self) -> f64 { Y_SCALES[self.scale_idx] }
    fn scale_up(&mut self) { if self.scale_idx+1<Y_SCALES.len(){self.scale_idx+=1;} }
    fn scale_down(&mut self) { if self.scale_idx>0{self.scale_idx-=1;} }
    fn auto_scale(&mut self) {
        let peak=self.bufs.iter().flat_map(|b|b.iter()).fold(0.0_f64,|a,&v|a.max(v.abs()));
        self.scale_idx=Y_SCALES.iter().position(|&s|s>=peak*1.1).unwrap_or(Y_SCALES.len()-1);
    }
}

fn smooth_signal(data: &[(f64,f64)], window: usize) -> Vec<(f64,f64)> {
    if data.len()<3||window<2{return data.to_vec();}
    let half=window/2;
    data.iter().enumerate().map(|(i,&(x,_))|{
        let s=i.saturating_sub(half); let e=(i+half+1).min(data.len());
        (x, data[s..e].iter().map(|&(_,y)|y).sum::<f64>()/(e-s) as f64)
    }).collect()
}

fn spinner_str() -> &'static str {
    let ms=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    SPINNER[(ms/100) as usize%SPINNER.len()]
}

// ── Interactive simulator ─────────────────────────────────────────────────────

#[cfg(feature = "simulate")]
fn spawn_interactive_simulator(app: Arc<Mutex<App>>) {
    use sim::*;
    tokio::spawn(async move {
        let pkt_interval = Duration::from_secs_f64(1.0/EEG_HZ);
        let mut ticker = tokio::time::interval(pkt_interval);
        let dt = 1.0/EEG_HZ;
        let (mut t, mut seq) = (0.0_f64, 0u64);
        let num_ch = 14usize;
        loop {
            ticker.tick().await;
            let mut s = app.lock().unwrap();
            s.sim.tick();
            if s.paused { t+=dt; seq+=1; continue; }
            let samples: Vec<f64> = (0..num_ch).map(|ch| s.sim.gen_eeg_sample(t,ch)).collect();
            s.push_eeg(&samples);
            if t > s.sim.artifact_end && s.sim.artifact != ArtifactKind::None { s.sim.artifact = ArtifactKind::None; }
            if seq%128==0 {
                let m = &s.sim.cur_metrics;
                s.metrics = Some(MetricsData{ values: vec![1.0,m[0],1.0,m[1],m[2],1.0,m[3],1.0,m[4],1.0,m[5],1.0,m[6]], time:t });
                let mut powers = Vec::with_capacity(num_ch*5);
                for ch in 0..num_ch { powers.extend_from_slice(&s.sim.gen_band_power(ch)); }
                s.band_power = Some(BandPowerData{powers,time:t});
                s.battery = Some(s.sim.battery);
                s.signal = Some(1.0);
                s.mc_action = Some(MC_ACTIONS[s.sim.mc_idx].to_string());
                s.mc_power = Some(s.sim.mc_power);
                s.fe_action = Some(FE_ACTIONS[s.sim.fe_idx].to_string());
                s.fe_power = Some(s.sim.fe_power);
            }
            t+=dt; seq+=1;
        }
    });
}

// ══════════════════════════════════════════════════════════════════════════════
//  Rendering
// ══════════════════════════════════════════════════════════════════════════════

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let root = Layout::vertical([Constraint::Length(3),Constraint::Min(0),Constraint::Length(3)]).split(area);
    draw_header(frame, root[0], app);
    match app.view {
        ViewMode::Eeg => draw_eeg_charts(frame, root[1], app),
        ViewMode::Metrics => draw_metrics(frame, root[1], app),
        ViewMode::BandPower => draw_band_power(frame, root[1], app),
        #[cfg(feature = "simulate")]
        ViewMode::SimControl => draw_sim_control(frame, root[1], app),
    }
    draw_footer(frame, root[2], app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let status = if !app.simulated {
        if app.connected { ("● Connected".into(), Color::Green) }
        else { (format!("{} Connecting…",spinner_str()), Color::Yellow) }
    } else {
        #[cfg(feature = "simulate")]
        { let n=app.sim.brain_state.name(); (format!("◆ SIM: {n}"), app.sim.brain_state.color()) }
        #[cfg(not(feature = "simulate"))]
        { ("◆ Simulated".into(), Color::Cyan) }
    };

    let bat=app.battery.map(|b|format!("Bat {b:.0}%")).unwrap_or("Bat N/A".into());
    let rate=format!("{:.1} pkt/s",app.pkt_rate());
    let scale=format!("±{:.0} µV",app.y_range());
    let total=format!("{}K smp",app.total_samples/1_000);
    let mc=app.mc_action.as_deref().map(|a|{let p=app.mc_power.unwrap_or(0.0);format!("MC:{a}({p:.2})")}).unwrap_or_default();
    let fe=app.fe_action.as_deref().map(|a|{let p=app.fe_power.unwrap_or(0.0);format!("FE:{a}({p:.2})")}).unwrap_or_default();

    #[allow(unused_mut)]
    let mut extra = String::new();
    #[cfg(feature = "simulate")]
    if app.simulated { extra = format!("noise={:.0}% gain={:.1}x", app.sim.noise_level*100.0, app.sim.gain); }

    #[cfg(feature = "raw")]
    {
        if !app.simulated {
            if let Some(dbg) = &app.raw_debug {
                let key = if dbg.key.len() > 24 {
                    format!("{}…", &dbg.key[..24])
                } else {
                    dbg.key.clone()
                };
                extra = format!(
                    "raw rx/dec/fail/tmo={}/{}/{}/{} ch={}/{} key={}",
                    dbg.rx,
                    dbg.dec,
                    dbg.fail,
                    dbg.timeout,
                    app.raw_last_decoded_channels,
                    app.num_channels,
                    key
                );
            }
        }
    }

    let line = Line::from(vec![
        Span::styled(" EMOTIV ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        sep(), Span::styled(status.0, Style::default().fg(status.1).add_modifier(Modifier::BOLD)),
        sep(), Span::styled(app.view.label(), Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD)),
        sep(), Span::styled(bat, Style::default().fg(Color::White)),
        sep(), Span::styled(rate, Style::default().fg(Color::White)),
        sep(), Span::styled(scale, Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)),
        sep(), Span::styled(total, Style::default().fg(Color::DarkGray)),
        sep(), Span::styled(mc, Style::default().fg(Color::LightGreen)),
        Span::raw(" "), Span::styled(fe, Style::default().fg(Color::LightMagenta)),
        sep(), Span::styled(extra, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line).block(Block::default().borders(Borders::ALL)), area);
}

#[inline] fn sep<'a>()->Span<'a>{Span::styled(" │ ",Style::default().fg(Color::DarkGray))}
#[inline] fn key_span(s:&str)->Span<'_>{Span::styled(s,Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))}

fn draw_eeg_charts(frame: &mut Frame, area: Rect, app: &App) {
    let n=app
        .num_channels
        .max(app.channel_labels.len())
        .max(1)
        .min(MAX_CHANNELS);
    let constraints:Vec<Constraint>=(0..n).map(|_|Constraint::Ratio(1,n as u32)).collect();
    let rows=Layout::vertical(constraints).split(area);
    let y_range=app.y_range();
    for ch in 0..n {
        let data:Vec<(f64,f64)>=app.bufs[ch].iter().enumerate().map(|(i,&v)|(i as f64/EEG_HZ,v.clamp(-y_range,y_range))).collect();
        let label=app.channel_labels.get(ch).cloned().unwrap_or_else(||{
            if n==14{EPOC_CHANNEL_NAMES.get(ch).unwrap_or(&"?").to_string()}
            else if n==5{INSIGHT_CHANNEL_NAMES.get(ch).unwrap_or(&"?").to_string()}
            else{format!("Ch{ch}")}
        });
        let color=COLORS[ch%COLORS.len()]; let dim_color=DIM_COLORS[ch%DIM_COLORS.len()];
        let smoothed:Vec<(f64,f64)>=if app.smooth{smooth_signal(&data,SMOOTH_WINDOW)}else{vec![]};
        let datasets:Vec<Dataset>=if app.smooth{vec![
            Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(dim_color)).data(&data),
            Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(color)).data(&smoothed),
        ]}else{vec![Dataset::default().marker(symbols::Marker::Braille).graph_type(GraphType::Line).style(Style::default().fg(color)).data(&data)]};

        let buf=&app.bufs[ch];
        let (min_v,max_v,rms_v)=if buf.is_empty(){(0.0,0.0,0.0)}else{
            let min=buf.iter().copied().fold(f64::INFINITY,f64::min);
            let max=buf.iter().copied().fold(f64::NEG_INFINITY,f64::max);
            (min,max,(buf.iter().map(|&v|v*v).sum::<f64>()/buf.len() as f64).sqrt())
        };
        #[cfg(feature = "raw")]
        let channel_na = !app.simulated && ch >= app.raw_last_decoded_channels && app.raw_last_decoded_channels > 0;
        #[cfg(not(feature = "raw"))]
        let channel_na = false;

        let clipping=(max_v>y_range||min_v< -y_range) && !channel_na;
        let clip_tag=if clipping{" [CLIP]"}else{""};
        let smooth_tag=if app.smooth{" [S]"}else{""};
        let na_tag=if channel_na{" [N/A]"}else{""};
        let title=format!(" {label}  {min_v:+.0}/{max_v:+.0} rms:{rms_v:.0}{na_tag}{clip_tag}{smooth_tag} ");
        let border_color=if clipping{Color::Red}else{color};
        let y_labels:Vec<String>=[-1.0,0.0,1.0].iter().map(|&f|format!("{:+.0}",f*y_range)).collect();

        frame.render_widget(Chart::new(datasets)
            .block(Block::default().title(Span::styled(title,Style::default().fg(color).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL).border_style(Style::default().fg(border_color)))
            .x_axis(Axis::default().bounds([0.0,WINDOW_SECS]).labels(vec!["0s".into(),format!("{:.0}s",WINDOW_SECS)]).style(Style::default().fg(Color::DarkGray)))
            .y_axis(Axis::default().bounds([-y_range,y_range]).labels(y_labels).style(Style::default().fg(Color::DarkGray))),
            rows[ch]);
    }
}

fn draw_metrics(frame: &mut Frame, area: Rect, app: &App) {
    let met=app.metrics.as_ref();
    let labels=["Engagement","Excitement","Lex.Excitement","Stress","Relaxation","Interest","Focus"];
    let indices=[1,3,4,6,8,10,12];
    let constraints:Vec<Constraint>=labels.iter().map(|_|Constraint::Ratio(1,labels.len() as u32)).collect();
    let rows=Layout::vertical(constraints).split(area);
    for(i,(&label,&idx))in labels.iter().zip(indices.iter()).enumerate(){
        let val=met.and_then(|m|m.values.get(idx)).copied().unwrap_or(0.0);
        let color=COLORS[i%COLORS.len()];
        let w=((area.width as f64-25.0)*val).max(0.0) as usize;
        let bar="█".repeat(w);
        let empty="░".repeat(((area.width as f64-25.0)-w as f64).max(0.0) as usize);
        frame.render_widget(Paragraph::new(format!(" {label:<16} {val:.3}  {bar}{empty}"))
            .style(Style::default().fg(color))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(color))),rows[i]);
    }
}

fn draw_band_power(frame: &mut Frame, area: Rect, app: &App) {
    let bp=app.band_power.as_ref();
    let band_names=["Theta","Alpha","BetaL","BetaH","Gamma"];
    let constraints:Vec<Constraint>=band_names.iter().map(|_|Constraint::Ratio(1,band_names.len() as u32)).collect();
    let rows=Layout::vertical(constraints).split(area);
    for(b,&band_name)in band_names.iter().enumerate(){
        let color=COLORS[b%COLORS.len()];
        let n_ch=app.num_channels.max(1);
        let values:Vec<f64>=(0..n_ch).map(|ch|bp.and_then(|d|d.powers.get(ch*5+b)).copied().unwrap_or(0.0)).collect();
        let max_val=values.iter().fold(0.0_f64,|a,&v|a.max(v));
        let avg_val=if values.is_empty(){0.0}else{values.iter().sum::<f64>()/values.len() as f64};
        let w=((area.width as f64-40.0)*(avg_val/10.0).min(1.0)).max(0.0) as usize;
        let bar="█".repeat(w);
        frame.render_widget(Paragraph::new(format!(" {band_name:<6} avg={avg_val:.2}  max={max_val:.2}  {bar}"))
            .style(Style::default().fg(color))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(color))),rows[b]);
    }
}

#[cfg(feature = "simulate")]
fn draw_sim_control(frame: &mut Frame, area: Rect, app: &App) {
    use sim::*;
    let rows=Layout::vertical([Constraint::Length(10),Constraint::Length(8),Constraint::Length(8),Constraint::Min(0)]).split(area);

    // Brain state
    let states=[(BrainState::Relaxed,"1","Strong alpha, low beta. Eyes closed."),(BrainState::Focused,"2","Strong beta. Concentrated task."),
        (BrainState::Excited,"3","Strong beta+gamma. High arousal."),(BrainState::Drowsy,"4","Strong theta. Falling asleep."),
        (BrainState::Meditative,"5","Strong alpha+theta. Deep relaxation.")];
    let mut lines:Vec<Line>=vec![Line::from(Span::styled(" Brain State",Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),Line::from("")];
    for(state,k,desc)in &states{
        let active=*state==app.sim.brain_state;
        let marker=if active{"▶ "}else{"  "};
        let color=if active{state.color()}else{Color::DarkGray};
        let(th,al,be,ga)=state.band_amplitudes();
        let key_label = format!("[{k}]");
        lines.push(Line::from(vec![
            Span::styled(format!(" {marker}"),Style::default().fg(color)),
            Span::styled(key_label,Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {:<12}",state.name()),Style::default().fg(color).add_modifier(if active{Modifier::BOLD}else{Modifier::empty()})),
            Span::styled(format!("θ={th:.0} α={al:.0} β={be:.0} γ={ga:.0}  "),Style::default().fg(color)),
            Span::styled(*desc,Style::default().fg(Color::DarkGray)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::White))),rows[0]);

    // Signal params
    let np=app.sim.noise_level*100.0; let g=app.sim.gain;
    let nw=(np/100.0*30.0) as usize; let gw=((g/3.0).min(1.0)*30.0) as usize;
    let sig_lines=vec![
        Line::from(Span::styled(" Signal Parameters",Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),Line::from(""),
        Line::from(vec![Span::raw("  "),key_span("[n]"),Span::raw("↓ "),key_span("[N]"),Span::raw("↑  "),
            Span::styled(format!("Noise: {np:5.1}%  "),Style::default().fg(Color::LightCyan)),
            Span::styled("█".repeat(nw),Style::default().fg(Color::LightCyan)),Span::styled("░".repeat(30-nw),Style::default().fg(Color::DarkGray))]),
        Line::from(vec![Span::raw("  "),key_span("[g]"),Span::raw("↓ "),key_span("[G]"),Span::raw("↑  "),
            Span::styled(format!("Gain:  {g:5.2}x  "),Style::default().fg(Color::LightYellow)),
            Span::styled("█".repeat(gw),Style::default().fg(Color::LightYellow)),Span::styled("░".repeat(30-gw),Style::default().fg(Color::DarkGray))]),
        Line::from(""),
        Line::from(vec![Span::raw("  "),key_span("[b]"),Span::styled(" Eye blink  ",Style::default().fg(Color::LightRed)),
            key_span("[j]"),Span::styled(" Jaw clench",Style::default().fg(Color::LightRed)),
            if app.sim.artifact!=ArtifactKind::None{Span::styled("  ⚡ ARTIFACT",Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))}else{Span::raw("")}]),
    ];
    frame.render_widget(Paragraph::new(sig_lines).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::LightCyan))),rows[1]);

    // BCI
    let mc=MC_ACTIONS[app.sim.mc_idx]; let mp=app.sim.mc_power;
    let fe=FE_ACTIONS[app.sim.fe_idx]; let fp=app.sim.fe_power;
    let bci_lines=vec![
        Line::from(Span::styled(" BCI Simulation",Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),Line::from(""),
        Line::from(vec![Span::raw("  "),key_span("[m]"),Span::raw(" Mental cmd:  "),
            Span::styled(format!("{mc:<10}"),Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
            Span::styled(format!("power={mp:.2}"),Style::default().fg(Color::Green)),
            Span::styled(format!("  [{}]",MC_ACTIONS.join(" → ")),Style::default().fg(Color::DarkGray))]),
        Line::from(vec![Span::raw("  "),key_span("[f]"),Span::raw(" Facial expr: "),
            Span::styled(format!("{fe:<10}"),Style::default().fg(Color::LightMagenta).add_modifier(Modifier::BOLD)),
            Span::styled(format!("power={fp:.2}"),Style::default().fg(Color::Magenta)),
            Span::styled(format!("  [{}]",FE_ACTIONS.join(" → ")),Style::default().fg(Color::DarkGray))]),
    ];
    frame.render_widget(Paragraph::new(bci_lines).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::LightGreen))),rows[2]);

    // Help
    let help=vec![Line::from(""),
        Line::from(Span::styled("  [Tab] cycle views   [1-5] brain state   [b/j] artifacts   [m/f] BCI   [n/N g/G] signal params",Style::default().fg(Color::DarkGray))),
        Line::from(Span::styled("  Brain state changes smoothly interpolate EEG waveform, band power, and performance metrics.",Style::default().fg(Color::DarkGray))),
    ];
    frame.render_widget(Paragraph::new(help).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray))),rows[3]);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let pause_span=if app.paused{Span::styled("  ⏸ PAUSED",Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))}else{Span::raw("")};
    let mut kv:Vec<Span>=vec![Span::raw(" "), key_span("[Tab]"),Span::raw("View ")];
    #[cfg(feature = "simulate")]
    if app.simulated { kv.extend_from_slice(&[key_span("[1-5]"),Span::raw("State ")]); }
    kv.extend_from_slice(&[key_span("[+/-]"),Span::raw("Scale "),key_span("[a]"),Span::raw("Auto "),
        key_span("[v]"),Span::raw(if app.smooth{"Raw "}else{"Smth "}),key_span("[p]"),Span::raw("Pause "),key_span("[c]"),Span::raw("Clear ")]);
    #[cfg(feature = "simulate")]
    if app.simulated { kv.extend_from_slice(&[key_span("[b]"),Span::raw("Blnk "),key_span("[j]"),Span::raw("Jaw "),
        key_span("[m]"),Span::raw("MC "),key_span("[f]"),Span::raw("FE ")]); }
    kv.extend_from_slice(&[key_span("[q]"),Span::raw("Quit"),pause_span]);

    let signal_str=app.signal.map(|s|format!("Sig:{s:.1}")).unwrap_or_default();
    #[allow(unused_mut)]
    let mut second_vec:Vec<Span>=vec![Span::raw(" "),Span::styled(signal_str,Style::default().fg(Color::Cyan)),
        Span::raw("  "),Span::styled(format!("Ch:{}",app.num_channels),Style::default().fg(Color::DarkGray))];
    #[cfg(feature = "simulate")]
    if app.simulated {
        second_vec.push(Span::styled(
            format!("  {}  θ={:.0} α={:.0} β={:.0} γ={:.0}",
                app.sim.brain_state.name(),app.sim.cur_theta,app.sim.cur_alpha,app.sim.cur_beta,app.sim.cur_gamma),
            Style::default().fg(app.sim.brain_state.color())));
    }

    frame.render_widget(Paragraph::new(vec![Line::from(kv),Line::from(second_vec)])
        .block(Block::default().borders(Borders::ALL)),area);
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    use std::io::IsTerminal as _;
    if !io::stdout().is_terminal() { eprintln!("Error: emotiv-tui requires a real terminal."); std::process::exit(1); }

    { use std::fs::File;
      let p=std::env::temp_dir().join("emotiv-tui.log");
      if let Ok(f)=File::create(&p){env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).target(env_logger::Target::Pipe(Box::new(f))).init();log::info!("Logging to {}",p.display());}}

    let args: Vec<String> = std::env::args().collect();
    let simulate = args.iter().any(|a| a == "--simulate");
    let raw_mode = args.iter().any(|a| a == "--raw");
    let raw_index = arg_value(&args, "--raw-index").and_then(|v| v.parse::<usize>().ok());
    let raw_target = arg_value(&args, "--raw-device")
        .or_else(|| std::env::var("EMOTIV_RAW_DEVICE").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if simulate && raw_mode {
        eprintln!("Error: --simulate and --raw cannot be used together.");
        std::process::exit(1);
    }

    if !raw_mode && (raw_index.is_some() || raw_target.is_some()) {
        eprintln!("Error: --raw-index/--raw-device require --raw.");
        std::process::exit(1);
    }

    #[cfg(not(feature = "simulate"))]
    if simulate {
        eprintln!("Error: --simulate requires the `simulate` feature.");
        eprintln!("  cargo run --bin emotiv-tui --features simulate -- --simulate");
        std::process::exit(1);
    }

    #[cfg(not(feature = "raw"))]
    if raw_mode {
        eprintln!("Error: --raw requires the `raw` feature.");
        eprintln!("  cargo run --bin emotiv-tui --features raw -- --raw");
        std::process::exit(1);
    }

    let app = Arc::new(Mutex::new(App::new(simulate)));
    let start_time = Instant::now();

    if simulate {
        #[cfg(feature = "simulate")]
        {
            let mut s = app.lock().unwrap();
            s.channel_labels = EPOC_CHANNEL_NAMES.iter().map(|n|n.to_string()).collect();
            s.connected = true;
            drop(s);
            spawn_interactive_simulator(Arc::clone(&app));
        }
        #[cfg(not(feature = "simulate"))]
        unreachable!();
    } else if raw_mode {
        #[cfg(feature = "raw")]
        {
            let app_clone = Arc::clone(&app);
            let raw_index = raw_index;
            let raw_target = raw_target.clone();
            tokio::spawn(async move {
                let mut discovered = raw::discover_devices().await;
                if matches!(discovered, Ok(ref d) if d.is_empty()) {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    discovered = raw::discover_devices().await;
                }

                match discovered {
                    Ok(devices) if !devices.is_empty() => {
                        let selected = if let Some(target) = raw_target {
                            let target_norm = normalize_id(&target);
                            devices.iter().position(|d| {
                                d.name.eq_ignore_ascii_case(&target)
                                    || d.address.eq_ignore_ascii_case(&target)
                                    || d.ble_id.eq_ignore_ascii_case(&target)
                                    || d.ble_mac.as_deref().map(|m| m.eq_ignore_ascii_case(&target)).unwrap_or(false)
                                    || d.serial.eq_ignore_ascii_case(&target)
                                    || normalize_id(&d.name).contains(&target_norm)
                                    || normalize_id(&d.address).contains(&target_norm)
                                    || normalize_id(&d.ble_id).contains(&target_norm)
                                    || d.ble_mac
                                        .as_ref()
                                        .map(|m| normalize_id(m).contains(&target_norm))
                                        .unwrap_or(false)
                                    || normalize_id(&d.serial).contains(&target_norm)
                            })
                        } else if let Some(index) = raw_index {
                            Some(index)
                        } else {
                            select_best_device(&devices)
                                .and_then(|best| devices.iter().position(|d| d.ble_id == best.ble_id && d.address == best.address))
                        }
                        .unwrap_or(0);

                        if selected >= devices.len() {
                            log::error!(
                                "Requested raw device index {} out of range (found {}).",
                                selected,
                                devices.len()
                            );
                            return;
                        }

                        let device = devices[selected].clone();
                        log::info!(
                            "Using raw device [{}]: {} | id={} | mac={} | serial={}",
                            selected,
                            device.name,
                            device.ble_id,
                            device.ble_mac.as_deref().unwrap_or("n/a"),
                            device.serial
                        );
                        {
                            let mut s = app_clone.lock().unwrap();
                            s.channel_labels = device
                                .model
                                .channels()
                                .into_iter()
                                .map(|x| x.to_string())
                                .collect();
                            s.num_channels = s.channel_labels.len().min(MAX_CHANNELS);
                        }

                        match raw::RawDevice::from_info(device).connect().await {
                            Ok((mut rx, handle)) => {
                                {
                                    let mut s = app_clone.lock().unwrap();
                                    s.connected = true;
                                }
                                let mut debug_tick = tokio::time::interval(std::time::Duration::from_millis(500));
                                loop {
                                    tokio::select! {
                                        _ = debug_tick.tick() => {
                                            let stats = handle.debug_stats().await;
                                            let mut s = app_clone.lock().unwrap();
                                            s.raw_debug = Some(RawDebugInfo {
                                                rx: stats.received_notifications,
                                                dec: stats.decoded_packets,
                                                fail: stats.decrypt_failures,
                                                timeout: stats.timeout_count,
                                                key: stats.active_serial_candidate.unwrap_or_else(|| "-".to_string()),
                                            });
                                        }
                                        maybe_data = rx.recv() => {
                                            match maybe_data {
                                                Some(data) => {
                                                    let mut s = app_clone.lock().unwrap();
                                                    let decoded_channels = data.eeg_uv.len().min(MAX_CHANNELS);
                                                    s.raw_last_decoded_channels = decoded_channels;
                                                    let labeled_channels = s.channel_labels.len().min(MAX_CHANNELS);
                                                    if labeled_channels > 0 {
                                                        s.num_channels = labeled_channels;
                                                    } else if decoded_channels > s.num_channels {
                                                        s.num_channels = decoded_channels;
                                                    }
                                                    s.push_eeg(&data.eeg_uv);
                                                    s.signal = Some(data.signal_quality as f64);
                                                }
                                                None => break,
                                            }
                                        }
                                    }
                                }
                                let mut s = app_clone.lock().unwrap();
                                s.connected = false;
                            }
                            Err(err) => {
                                log::error!("Raw BLE connection failed: {}", err);
                            }
                        }
                    }
                    Ok(_) => {
                        log::error!("No raw BLE devices found");
                    }
                    Err(err) => {
                        log::error!("Raw BLE discovery failed: {}", err);
                    }
                }
            });
        }
        #[cfg(not(feature = "raw"))]
        unreachable!();
    } else {
        let app_clone = Arc::clone(&app);
        let client_id = std::env::var("EMOTIV_CLIENT_ID").unwrap_or_else(|_|"your_client_id".into());
        let client_secret = std::env::var("EMOTIV_CLIENT_SECRET").unwrap_or_else(|_|"your_client_secret".into());
        tokio::spawn(async move {
            let config = emotiv::client::CortexClientConfig{client_id,client_secret,..Default::default()};
            let client = emotiv::client::CortexClient::new(config);
            match client.connect().await {
                Ok((mut rx,handle)) => {
                    while let Some(ev) = rx.recv().await {
                        let mut s = app_clone.lock().unwrap();
                        match ev {
                            CortexEvent::Connected => s.connected=true,
                            CortexEvent::SessionCreated(_) => { drop(s); let _=handle.subscribe(&["eeg","mot","dev","met","pow"]).await; continue; }
                            CortexEvent::Eeg(d) => s.push_eeg(&d.samples),
                            CortexEvent::Dev(d) => { s.battery=Some(d.battery_percent); s.signal=Some(d.signal); }
                            CortexEvent::Metrics(d) => s.metrics=Some(d),
                            CortexEvent::BandPower(d) => s.band_power=Some(d),
                            CortexEvent::MentalCommand(d) => { s.mc_action=Some(d.action); s.mc_power=Some(d.power); }
                            CortexEvent::FacialExpression(d) => { s.fe_action=Some(d.eye_action.clone()); s.fe_power=Some(d.upper_power); }
                            CortexEvent::DataLabels(l) => { if l.stream_name=="eeg"{s.channel_labels=l.labels;} }
                            CortexEvent::Disconnected => { s.connected=false; break; }
                            _ => {}
                        }
                    }
                }
                Err(e) => { log::error!("Connection failed: {e}"); }
            }
        });
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let tick = Duration::from_millis(33);

    loop {
        { let s=app.lock().unwrap(); terminal.draw(|f|draw(f,&s))?; }

        if event::poll(tick)? {
            if let Event::Key(ke) = event::read()? {
                let mut s = app.lock().unwrap();
                let is_sim = s.simulated;
                let _t = start_time.elapsed().as_secs_f64();

                match ke.code {
                    KeyCode::Char('q')|KeyCode::Esc => break,
                    KeyCode::Char('c') if ke.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Tab => { s.view = s.view.next(is_sim); }
                    KeyCode::Char('+')|KeyCode::Char('=') => s.scale_up(),
                    KeyCode::Char('-') => s.scale_down(),
                    KeyCode::Char('a') => s.auto_scale(),
                    KeyCode::Char('v') => s.smooth = !s.smooth,
                    KeyCode::Char('p') => s.paused = true,
                    KeyCode::Char('r') => s.paused = false,
                    KeyCode::Char('c') => s.clear(),

                    #[cfg(feature = "simulate")]
                    KeyCode::Char('1') if is_sim => s.sim.set_brain_state(sim::BrainState::Relaxed),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('2') if is_sim => s.sim.set_brain_state(sim::BrainState::Focused),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('3') if is_sim => s.sim.set_brain_state(sim::BrainState::Excited),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('4') if is_sim => s.sim.set_brain_state(sim::BrainState::Drowsy),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('5') if is_sim => s.sim.set_brain_state(sim::BrainState::Meditative),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('b') if is_sim => s.sim.inject_artifact(sim::ArtifactKind::Blink, _t),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('j') if is_sim => s.sim.inject_artifact(sim::ArtifactKind::JawClench, _t),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('m') if is_sim => s.sim.cycle_mc(),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('f') if is_sim => s.sim.cycle_fe(),
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('n') if is_sim => { s.sim.noise_level=(s.sim.noise_level-0.1).max(0.0); }
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('N') if is_sim => { s.sim.noise_level=(s.sim.noise_level+0.1).min(2.0); }
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('g') if is_sim => { s.sim.gain=(s.sim.gain-0.1).max(0.1); }
                    #[cfg(feature = "simulate")]
                    KeyCode::Char('G') if is_sim => { s.sim.gain=(s.sim.gain+0.1).min(5.0); }

                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
