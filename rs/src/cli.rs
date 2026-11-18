use std::fmt::Display;
use std::ops::RangeInclusive;
use std::process::exit;
use std::str::FromStr;
use std::sync::OnceLock;

use crate::clock::ClockStyle;
use crate::screen::ColorMode;
use crate::term;
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Live,
    Still,
    Print,
}

#[derive(Debug)]
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

impl Default for Options {
    fn default() -> Self {
        Options {
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
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Stop {
    Print(String),
    Fail(String),
}

fn number<T: FromStr + PartialOrd + Display>(
    flag: &str,
    value: &str,
    range: RangeInclusive<T>,
) -> Result<T, Stop> {
    match value.parse() {
        Ok(n) if range.contains(&n) => Ok(n),
        _ => Err(Stop::Fail(format!(
            "{flag} {} {} {} {}",
            t("must be between", "要介於"),
            range.start(),
            t("and", "到"),
            range.end()
        ))),
    }
}

pub fn parse() -> Options {
    parse_from(std::env::args().skip(1)).unwrap_or_else(|stop| match stop {
        Stop::Print(text) => {
            println!("{text}");
            exit(0);
        }
        Stop::Fail(msg) => fail(&msg),
    })
}

pub fn parse_from(args: impl IntoIterator<Item = String>) -> Result<Options, Stop> {
    let mut o = Options::default();
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        let (flag, mut inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg, None),
        };
        if flag == "--clock" {
            let style = inline.or_else(|| args.next_if(|a| ClockStyle::parse(a).is_some()));
            let style = style.as_deref().unwrap_or("block");
            o.clock = Some(ClockStyle::parse(style).ok_or_else(|| {
                Stop::Fail(format!(
                    "{}: {style} (see --styles)",
                    t("unknown clock style", "不認得的時鐘字型")
                ))
            })?);
            continue;
        }
        let mut value = || {
            inline
                .take()
                .or_else(|| args.next())
                .ok_or_else(|| Stop::Fail(format!("{flag} {}", t("needs a value", "需要一個值"))))
        };
        match flag.as_str() {
            "-h" | "--help" => return Err(Stop::Print(t(HELP_EN, HELP_ZH).into())),
            "-V" | "--version" => {
                return Err(Stop::Print(format!("sharks {}", env!("CARGO_PKG_VERSION"))));
            }
            "-l" | "--list" => {
                let list = SharkKind::ALL.map(|k| format!("{:<12} {}", k.name(), k.label(zh())));
                return Err(Stop::Print(list.join("\n")));
            }
            "--styles" => o.list_styles = true,
            "-k" | "--sharks" => o.kinds = Some(parse_kinds(&value()?)?),
            "-n" | "--count" => o.count = number(&flag, &value()?, 1..=MAX_SHARKS)?,
            "--fish" => o.fish = Some(number(&flag, &value()?, 1..=1000)?),
            "--seed" => o.seed = Some(number(&flag, &value()?, 0..=u32::MAX)?),
            "--speed" => o.speed = number(&flag, &value()?, 0.1..=5.0)?,
            "--rain" => o.rain = true,
            "--calm" => o.calm = true,
            "--seconds" => o.seconds = true,
            "-m" | "--message" => {
                o.message = Some(
                    value()?
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .collect(),
                )
            }
            "-f" | "--fps" => o.fps = number(&flag, &value()?, 1.0..=120.0)?,
            "--color" => {
                let v = value()?;
                o.color = match v.as_str() {
                    "auto" => None,
                    v => Some(ColorMode::parse(v).ok_or_else(|| {
                        Stop::Fail(format!(
                            "--color: auto, truecolor, 256, 16 {} mono ({v}?)",
                            t("or", "或")
                        ))
                    })?),
                };
            }
            "--ascii" => o.ascii = true,
            "--aspect" => o.aspect = Some(number(&flag, &value()?, term::ASPECT)?),
            "-S" | "--screensaver" => o.screensaver = true,
            "--still" => o.mode = Mode::Still,
            "--print" => o.mode = Mode::Print,
            _ if zh() => return Err(Stop::Fail(format!("不認得的選項：{flag}（用 -h 看說明）"))),
            _ => return Err(Stop::Fail(format!("unknown option: {flag} (see -h)"))),
        }
    }
    Ok(o)
}

const MAX_SHARKS: usize = 12;

pub fn parse_kinds(list: &str) -> Result<Vec<SharkKind>, Stop> {
    let mut kinds = Vec::new();
    let mut unknown = Vec::new();
    for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match SharkKind::parse(name) {
            Some(kind) => kinds.push(kind),
            None => unknown.push(name),
        }
    }
    let unknown = unknown.join(", ");
    match kinds.len() {
        _ if !unknown.is_empty() && zh() => Err(Stop::Fail(format!(
            "不認得的鯊魚種類：{unknown}（用 -l 看清單）"
        ))),
        _ if !unknown.is_empty() => Err(Stop::Fail(format!(
            "unknown shark kind: {unknown} (see -l)"
        ))),
        1..=MAX_SHARKS => Ok(kinds),
        _ => Err(Stop::Fail(format!(
            "-k: 1–{MAX_SHARKS} {}",
            t("sharks", "隻鯊魚")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, Stop> {
        parse_from(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn clock_style_with_or_without_equals() {
        assert_eq!(parse(&["--clock"]).unwrap().clock, Some(ClockStyle::Block));
        assert_eq!(
            parse(&["--clock=lcd"]).unwrap().clock,
            Some(ClockStyle::Lcd)
        );
        let o = parse(&["--clock", "dots", "--rain"]).unwrap();
        assert_eq!((o.clock, o.rain), (Some(ClockStyle::Dots), true));
        assert!(
            matches!(parse(&["--clock", "--rain"]), Ok(o) if o.clock == Some(ClockStyle::Block) && o.rain)
        );
        assert!(parse(&["--clock=nope"]).is_err());
    }

    #[test]
    fn rejects_bad_values() {
        for args in [
            &["-k", ""][..],
            &["-k", ","],
            &["-k", "tiger,whale"],
            &["-n", "13"],
            &["--fps", "0"],
            &["--color", "rgb"],
            &["-f"],
        ] {
            assert!(matches!(parse(args), Err(Stop::Fail(_))), "{args:?}");
        }
        assert!(parse(&["-k", &["mako"; 12].join(",")]).is_ok());
        assert!(parse(&["-k", &["mako"; 13].join(",")]).is_err());
    }

    #[test]
    fn message_loses_control_characters() {
        let o = parse(&["-m", "one\ntwo\x1b[31m"]).unwrap();
        assert_eq!(o.message.as_deref(), Some("one two [31m"));
    }

    #[test]
    fn colour_auto_and_modes() {
        assert_eq!(parse(&["--color", "auto"]).unwrap().color, None);
        assert_eq!(
            parse(&["--color=256"]).unwrap().color,
            Some(ColorMode::Ansi256)
        );
    }
}
