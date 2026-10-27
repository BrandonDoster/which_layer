//! `which_layer bench`: measures RAM in each window state, and how long it takes
//! to get the overlay on screen from hidden, from destroyed, and from a cold
//! process start, so we can pick between keep-alive and rebuild-on-demand.

use crate::{window, App};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

/// Wait after a state change before sampling memory, so DWM and the heap settle.
const SETTLE_MS: u64 = 1000;
/// Wait between timed operations.
const GAP_MS: u64 = 100;
const RUNS: usize = 20;
const COLD_RUNS: usize = 5;

enum Op {
    Create { timed: bool },
    Show,
    Hide,
    Destroy,
    Sample(String),
    Cold,
}

pub struct Bench {
    ops: VecDeque<(Op, u64)>,
    next: Instant,
    layout_args: Vec<String>,
    samples: Vec<(String, usize, usize)>,
    /// (ms until ShowWindow returned, ms until DWM composed it)
    from_hidden: Vec<(f64, f64)>,
    from_destroyed: Vec<(f64, f64)>,
    /// (ms until the child reported on screen, child working set, child private)
    cold: Vec<(f64, usize, usize)>,
}

impl Bench {
    pub fn new(layout_args: Vec<String>) -> Bench {
        let mut ops = VecDeque::new();
        let mut op = |o, wait| ops.push_back((o, wait));
        op(Op::Sample("startup, no window yet".into()), 0);
        for i in 1..=3 {
            op(Op::Create { timed: true }, SETTLE_MS);
            op(Op::Sample(format!("cycle {i}: created + shown")), 0);
            op(Op::Hide, SETTLE_MS);
            op(Op::Sample(format!("cycle {i}: hidden")), 0);
            op(Op::Show, SETTLE_MS);
            op(Op::Sample(format!("cycle {i}: shown again")), 0);
            op(Op::Destroy, SETTLE_MS);
            op(Op::Sample(format!("cycle {i}: destroyed")), 0);
        }
        op(Op::Create { timed: false }, GAP_MS);
        for _ in 0..RUNS {
            op(Op::Hide, GAP_MS);
            op(Op::Show, GAP_MS);
        }
        for _ in 0..RUNS {
            op(Op::Destroy, GAP_MS);
            op(Op::Create { timed: true }, GAP_MS);
        }
        op(Op::Destroy, GAP_MS);
        for _ in 0..COLD_RUNS {
            op(Op::Cold, GAP_MS);
        }
        println!("Running bench (~30s). The overlay will flash on and off.");
        Bench {
            ops,
            next: Instant::now(),
            layout_args,
            samples: Vec::new(),
            from_hidden: Vec::new(),
            from_destroyed: Vec::new(),
            cold: Vec::new(),
        }
    }

    /// Runs the next op when its time comes. Returns true when finished.
    pub fn tick(&mut self, app: &mut App, el: &ActiveEventLoop) -> bool {
        if Instant::now() >= self.next {
            let Some((op, wait)) = self.ops.pop_front() else {
                self.report();
                el.exit();
                return true;
            };
            self.run(op, app, el);
            self.next = Instant::now() + Duration::from_millis(wait);
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next));
        false
    }

    fn run(&mut self, op: Op, app: &mut App, el: &ActiveEventLoop) {
        match op {
            Op::Create { timed } => {
                let t = timed_display(|| app.create_show(el));
                if timed {
                    self.from_destroyed.push(t);
                }
            }
            // A layer change while hidden is the realistic reason to show, so
            // both paths include rendering a new layer.
            Op::Show => {
                app.apply_state(1 << ((app.top_layer() + 1) % app.layout.layers.len().min(32)));
                self.from_hidden.push(timed_display(|| app.show()));
            }
            Op::Hide => app.hide(),
            Op::Destroy => app.destroy(),
            Op::Sample(label) => {
                let (ws, private) = mem();
                self.samples.push((label, ws, private));
            }
            Op::Cold => self.cold.push(cold_start(&self.layout_args)),
        }
    }

    fn report(&self) {
        println!("\nMemory, this process        working set   private bytes");
        for (label, ws, private) in &self.samples {
            println!("  {label:<26} {:>11} {:>15}", mb(*ws), mb(*private));
        }
        println!("  (the window's surface is held by dwm.exe and isn't counted here)");

        println!("\nTime to display (ms)         ShowWindow returned   on screen (DwmFlush)");
        println!("                               min   med   max      min   med   max");
        let (first, rebuilds) = self.from_destroyed.split_first().expect("bench ran creates");
        row("hidden -> shown", &self.from_hidden);
        row("destroyed -> recreated", rebuilds);
        println!("  {:<26} {:>5.1}                  {:>5.1}", "first window ever", first.0, first.1);
        let cold: Vec<f64> = self.cold.iter().map(|c| c.0).collect();
        let (min, med, max) = stats(&cold);
        println!("  {:<26}                        {min:>5.1} {med:>5.1} {max:>5.1}", "cold process start");
        if let Some((_, ws, private)) = self.cold.last() {
            println!("  fresh process RAM when shown: working set {}, private {}", mb(*ws), mb(*private));
        }
        println!("  (on-screen times include waiting for the next vsync, up to one frame)");
    }
}

/// Times `f`, then how long until DWM has composed the result.
fn timed_display(f: impl FnOnce()) -> (f64, f64) {
    let t0 = Instant::now();
    f();
    let shown = ms(t0.elapsed());
    window::wait_for_compositor();
    (shown, ms(t0.elapsed()))
}

/// Spawns `which_layer probe`, which exits right after its first frame is on screen.
fn cold_start(layout_args: &[String]) -> (f64, usize, usize) {
    let exe = std::env::current_exe().expect("current exe");
    let t0 = Instant::now();
    let mut child = Command::new(exe)
        .arg("probe")
        .args(layout_args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn probe");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).expect("read probe output");
    let elapsed = ms(t0.elapsed());
    let _ = child.wait();
    let mut nums = line.split_whitespace().skip(1).map(|n| n.parse().unwrap_or(0));
    (elapsed, nums.next().unwrap_or(0), nums.next().unwrap_or(0))
}

/// (working set, private bytes) of this process.
pub fn mem() -> (usize, usize) {
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb);
        (c.WorkingSetSize, c.PrivateUsage)
    }
}

pub fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn row(label: &str, times: &[(f64, f64)]) {
    let (a0, a1, a2) = stats(&times.iter().map(|t| t.0).collect::<Vec<_>>());
    let (b0, b1, b2) = stats(&times.iter().map(|t| t.1).collect::<Vec<_>>());
    println!("  {label:<26} {a0:>5.1} {a1:>5.1} {a2:>5.1}    {b0:>5.1} {b1:>5.1} {b2:>5.1}");
}

/// (min, median, max)
fn stats(v: &[f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    (v[0], v[v.len() / 2], v[v.len() - 1])
}
