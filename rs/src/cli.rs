use std::fmt::Display;
use std::ops::RangeInclusive;
use std::process::exit;
use std::str::FromStr;
use std::sync::OnceLock;

use crate::clock::ClockStyle;
use crate::term::{self, ColorMode};
use crate::world::SharkKind;

const HELP_EN: &str = "sharks — ASCII sharks and fish schools in your terminal

Usage: sharks [options]

Scene
  -k, --sharks <kind,...>  Which sharks to show (default: different kinds at random)
  -n, --count <n>          How many random sharks, 1–12 (default 4)
      --fish <n>           How many fish, 1–1000 (default: by window size)
      --seed <n>           Repeat the same scene
      --speed <x>          Simulation speed, 0.1–5 (default 1)
      --rain               Raindrops dimple the water
      --calm               No shimmer or flicker on the sharks
  -l, --list               List the shark kinds

Clock and text
      --clock[=<style>]    Show the time: block shadow hash half dots big lcd
      --seconds            Include the seconds
  -m, --message <text>     Show a line of text
      --styles             Preview the clock styles

Display
  -f, --fps <n>            Frames per second, 1–120 (default 30)
      --color <mode>       auto, truecolor, 256, 16 or mono (default auto)
      --ascii              ASCII glyphs only
      --aspect <n>         Cell height / width (default: ask the terminal)
  -S, --screensaver        Any key quits
      --still              One still frame, redrawn only when the clock changes
      --print              Print one frame and exit, e.g. as a shell greeting
  -V, --version
  -h, --help

While running
  q, Esc             quit
  p                  pause
  Ctrl-Z             suspend
  mouse              fish flee the pointer; hover a shark to see its name
  click, space       ripple: the fish scatter, nearby sharks turn away
  right-click, f     drop food: a ball of sardines the sharks come for

Every shark gets its own random size. Messages follow the locale;
set SHARKS_LANG=zh-TW for Chinese.";

const HELP_ZH: &str = "sharks — 終端機裡的 ASCII 鯊魚與魚群

用法: sharks [選項]

場景
  -k, --sharks <種類,...>  指定要放哪些鯊魚（預設：隨機挑選不同種類）
  -n, --count <n>          隨機鯊魚的數量，1～12（預設 4）
      --fish <n>           魚的數量，1～1000（預設依視窗大小）
      --seed <n>           重現同一個場景
      --speed <x>          模擬速度，0.1～5（預設 1）
      --rain               雨滴落在水面
      --calm               鯊魚不閃爍、不變色
  -l, --list               列出所有鯊魚種類

時鐘與文字
      --clock[=<字型>]     顯示時間：block shadow hash half dots big lcd
      --seconds            顯示秒數
  -m, --message <文字>     顯示一行文字
      --styles             預覽所有時鐘字型

顯示
  -f, --fps <n>            每秒畫面數，1～120（預設 30）
      --color <模式>       auto、truecolor、256、16 或 mono（預設 auto）
      --ascii              只用 ASCII 字元
      --aspect <n>         字元格高寬比（預設向終端機查詢）
  -S, --screensaver        按任意鍵離開
      --still              靜止畫面，只在時鐘變動時重畫
      --print              印出一幀畫面後結束，例如當作登入畫面
  -V, --version
  -h, --help

執行中
  q、Esc             離開
  p                  暫停
  Ctrl-Z             暫停到背景
  滑鼠               魚群避開游標；停在鯊魚上會顯示名稱
  點擊、空白鍵       漣漪：魚群四散，附近的鯊魚轉向逃開
  右鍵、f            餵食：投下一群沙丁魚，鯊魚會聚過來

每隻鯊魚的大小都是隨機的。訊息語言依語系而定；設定 SHARKS_LANG=en 可改用英文。";

pub fn zh() -> bool {
    static ZH: OnceLock<bool> = OnceLock::new();
    *ZH.get_or_init(|| {
        ["SHARKS_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("zh"))
    })
}

pub fn t(en: &'static str, zh_text: &'static str) -> &'static str {
    if zh() { zh_text } else { en }
}

pub fn fail(msg: &str) -> ! {
    eprintln!("sharks: {msg}");
    exit(2);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Live,
    Still,
    Print,
}

pub struct Options {
    pub kinds: Option<Vec<SharkKind>>,
    pub count: usize,
    pub fish: Option<usize>,
    pub seed: Option<u32>,
    pub speed: f64,
    pub rain: bool,
    pub calm: bool,
    pub clock: Option<ClockStyle>,
    pub seconds: bool,
    pub message: Option<String>,
    pub fps: f64,
    pub color: Option<ColorMode>,
    pub ascii: bool,
    pub aspect: Option<f64>,
    pub screensaver: bool,
    pub mode: Mode,
    pub list_styles: bool,
}

fn number<T: FromStr + PartialOrd + Display>(
    flag: &str,
    value: &str,
    range: RangeInclusive<T>,
) -> T {
    match value.parse() {
        Ok(n) if range.contains(&n) => n,
        _ if zh() => fail(&format!(
            "{flag} 要介於 {} 到 {}",
            range.start(),
            range.end()
        )),
        _ => fail(&format!(
            "{flag} must be between {} and {}",
            range.start(),
            range.end()
        )),
    }
}

pub fn parse() -> Options {
    let mut o = Options {
        kinds: None,
        count: 4,
        fish: None,
        seed: None,
        speed: 1.0,
        rain: false,
        calm: false,
        clock: None,
        seconds: false,
        message: None,
        fps: 30.0,
        color: None,
        ascii: false,
        aspect: None,
        screensaver: false,
        mode: Mode::Live,
        list_styles: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, mut inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg, None),
        };
        if flag == "--clock" {
            let style = inline.take().unwrap_or_else(|| "block".into());
            o.clock = Some(ClockStyle::parse(&style).unwrap_or_else(|| {
                fail(&format!(
                    "{}: {style} (see --styles)",
                    t("unknown clock style", "不認得的時鐘字型")
                ))
            }));
            continue;
        }
        let mut value = || {
            inline
                .take()
                .or_else(|| args.next())
                .unwrap_or_else(|| fail(&format!("{flag} {}", t("needs a value", "需要一個值"))))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{}", t(HELP_EN, HELP_ZH));
                exit(0);
            }
            "-l" | "--list" => {
                for kind in SharkKind::ALL {
                    println!("{:<12} {}", kind.name(), kind.label(zh()));
                }
                exit(0);
            }
            "-V" | "--version" => {
                println!("sharks {}", env!("CARGO_PKG_VERSION"));
                exit(0);
            }
            "--styles" => o.list_styles = true,
            "-k" | "--sharks" => o.kinds = Some(parse_kinds(&value())),
            "-n" | "--count" => o.count = number(&flag, &value(), 1..=12),
            "--fish" => o.fish = Some(number(&flag, &value(), 1..=1000)),
            "--seed" => o.seed = Some(number(&flag, &value(), 0..=u32::MAX)),
            "--speed" => o.speed = number(&flag, &value(), 0.1..=5.0),
            "--rain" => o.rain = true,
            "--calm" => o.calm = true,
            "--seconds" => o.seconds = true,
            "-m" | "--message" => o.message = Some(value()),
            "-f" | "--fps" => o.fps = number(&flag, &value(), 1.0..=120.0),
            "--color" => {
                let v = value();
                o.color = ColorMode::parse(&v).unwrap_or_else(|| {
                    fail(&format!("--color: auto, truecolor, 256, 16, mono ({v}?)"))
                });
            }
            "--ascii" => o.ascii = true,
            "--aspect" => o.aspect = Some(number(&flag, &value(), term::ASPECT)),
            "-S" | "--screensaver" => o.screensaver = true,
            "--still" => o.mode = Mode::Still,
            "--print" => o.mode = Mode::Print,
            _ if zh() => fail(&format!("不認得的選項：{flag}（用 -h 看說明）")),
            _ => fail(&format!("unknown option: {flag} (see -h)")),
        }
    }
    o
}

pub fn parse_kinds(list: &str) -> Vec<SharkKind> {
    let (kinds, unknown): (Vec<_>, Vec<_>) = list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|n| SharkKind::parse(n).ok_or(n))
        .partition(Result::is_ok);
    if !unknown.is_empty() {
        let unknown: Vec<&str> = unknown.into_iter().filter_map(Result::err).collect();
        let unknown = unknown.join(", ");
        fail(&if zh() {
            format!("不認得的鯊魚種類：{unknown}（用 -l 看清單）")
        } else {
            format!("unknown shark kind: {unknown} (see -l)")
        });
    }
    kinds.into_iter().flatten().collect()
}
