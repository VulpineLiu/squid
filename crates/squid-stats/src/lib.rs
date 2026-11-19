//! Live system metrics — the iStat Menus half of squid.
//!
//! [`Sampler`] owns the stateful sysinfo handles. Call [`Sampler::refresh`]
//! once per UI tick (~1 Hz): every rate in a [`SystemSnapshot`] is derived from
//! the delta to the previous sample, so the *first* sample reports zero rates
//! and every later one is honest.

use serde::{Deserialize, Serialize};
use squid_core::{Bytes, Volume};
use squid_platform::process::StopError;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{
    DiskRefreshKind, DiskUsage, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System,
};

mod battery;
pub use battery::Battery;

/// Everything one tick knows about the machine.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub taken_at_ms: u64,
    pub cpu: CpuStats,
    pub memory: MemoryStats,
    pub network: NetRate,
    pub disk: DiskRate,
    /// GPU busy percent 0–100 when the OS exposes it (None on headless/VDI).
    pub gpu_usage: Option<f32>,
    pub uptime_secs: u64,
    pub processes: usize,
    pub top_processes: Vec<ProcessSample>,
    pub volumes: Vec<Volume>,
    pub battery: Option<Battery>,
    pub host: HostInfo,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuStats {
    /// 0.0–100.0 across all cores.
    pub total: f32,
    pub per_core: Vec<f32>,
    pub load: [f32; 3],
    pub physical_cores: usize,
    pub logical_cores: usize,
    pub brand: String,
    pub base_frequency_ghz: f64,
}

impl CpuStats {
    pub fn busy_cores(&self) -> f32 {
        self.total / 100.0 * self.logical_cores.max(1) as f32
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemoryStats {
    pub total: Bytes,
    pub used: Bytes,
    pub available: Bytes,
    /// RAM the machine ships with. `total` is what the OS may use after
    /// firmware/graphics carve-outs; `installed - total` is that carve-out.
    pub installed: Bytes,
    pub swap_total: Bytes,
    pub swap_used: Bytes,
    /// macOS keeps wired/compressed pages out of `used`; expose them when known.
    pub compressed: Option<Bytes>,
    pub wired: Option<Bytes>,
}

impl MemoryStats {
    pub fn percent_used(&self) -> u8 {
        if self.total.is_zero() {
            0
        } else {
            self.used.percent_of(self.total)
        }
    }

    /// Physical memory the firmware or the integrated GPU holds back from the OS.
    pub fn hardware_reserved(&self) -> Bytes {
        Bytes(self.installed.0.saturating_sub(self.total.0))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetRate {
    pub rx_per_sec: Bytes,
    pub tx_per_sec: Bytes,
    pub rx_total: Bytes,
    pub tx_total: Bytes,
    pub interfaces: usize,
    pub primary: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiskRate {
    pub read_per_sec: Bytes,
    pub write_per_sec: Bytes,
    /// 0.0–100.0, busiest physical disk.
    pub activity: f32,
    pub disks: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessSample {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: Bytes,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostInfo {
    pub os_name: String,
    pub os_version: String,
    pub hardware: String,
    pub gpu: String,
    pub hostname: String,
    pub user: String,
    pub arch: String,
}

impl HostInfo {
    pub fn gather() -> HostInfo {
        let platform = squid_platform::current();
        HostInfo {
            os_name: platform.display_name(),
            os_version: platform.os_version(),
            hardware: platform.hardware_model(),
            gpu: platform.graphics_adapter(),
            hostname: hostname(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_else(|_| "unknown".into()),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

#[cfg(unix)]
fn hostname() -> String {
    const BUF: usize = 256;
    let mut buf = vec![0u8; BUF];
    let name = unsafe {
        if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, BUF) == 0 {
            let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        } else {
            String::new()
        }
    };
    if name.is_empty() {
        "localhost".into()
    } else {
        name
    }
}

#[cfg(not(unix))]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".into())
}

/// Seconds → "3d 4h 12m".
pub fn human_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3_600;
    let minutes = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m {}s", secs % 60)
    }
}

// ─────────────────────────────── sampling ─────────────────────────────────

/// Stateful sampler. `System`, `Networks` and `Disks` all compute deltas from
/// their own previous state, so they must be kept alive between ticks.
pub struct Sampler {
    system: System,
    networks: Networks,
    disks: Disks,
    last: Instant,
    host: HostInfo,
    volumes: Vec<Volume>,
    volumes_at: Option<Instant>,
    battery: Option<Battery>,
    battery_at: Option<Instant>,
    had_first_sample: bool,
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    pub fn new() -> Sampler {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::everything())
                .with_memory(sysinfo::MemoryRefreshKind::everything())
                .with_processes(ProcessRefreshKind::nothing().with_cpu().with_memory()),
        );
        Sampler {
            networks: Networks::new_with_refreshed_list(),
            // Capacity comes from squid-platform's statvfs path. Asking
            // sysinfo for storage here uses CoreFoundation's
            // `AvailableCapacityForImportantUsage`, which enumerates every
            // CacheDelete provider on macOS and can prompt for media access.
            disks: Disks::new_with_refreshed_list_specifics(
                DiskRefreshKind::nothing().with_io_usage(),
            ),
            system,
            last: Instant::now(),
            host: HostInfo::gather(),
            volumes: Vec::new(),
            volumes_at: None,
            battery: None,
            battery_at: None,
            had_first_sample: false,
        }
    }

    /// Cheap first call: fills totals without claiming any rates.
    pub fn prime(&mut self) -> SystemSnapshot {
        let snapshot = self.refresh();
        SystemSnapshot {
            network: NetRate {
                interfaces: snapshot.network.interfaces,
                rx_total: snapshot.network.rx_total,
                tx_total: snapshot.network.tx_total,
                primary: snapshot.network.primary,
                ..Default::default()
            },
            disk: DiskRate {
                disks: snapshot.disk.disks,
                ..Default::default()
            },
            cpu: CpuStats {
                total: 0.0,
                ..snapshot.cpu
            },
            ..snapshot
        }
    }

    pub fn refresh(&mut self) -> SystemSnapshot {
        self.refresh_with(true)
    }

    /// `want_processes=false` is the widget-grade light refresh: it skips the
    /// per-pid CPU/memory walk (hundreds of Get-ProcessTimes calls on
    /// Windows) and the top-process sort, keeping only a cheap pid sweep for
    /// the process count. The always-on desktop widget pays this on every
    /// beat; only 实时监控 asks for the full table.
    pub fn refresh_with(&mut self, want_processes: bool) -> SystemSnapshot {
        let elapsed = self.last.elapsed().max(Duration::from_millis(80));
        self.last = Instant::now();

        self.system
            .refresh_cpu_specifics(sysinfo::CpuRefreshKind::nothing().with_cpu_usage());
        self.system
            .refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram().with_swap());
        if want_processes {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
        } else {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing(),
            );
        }
        self.networks.refresh(true);
        self.disks
            .refresh_specifics(true, DiskRefreshKind::nothing().with_io_usage());

        let seconds = elapsed.as_secs_f64();
        let cpu = cpu_stats(&self.system);
        let memory = memory_stats(&self.system);
        let network = net_rates(&self.networks, seconds);
        let disk = disk_rates(&self.disks, seconds);
        let top_processes = if want_processes {
            top_processes(&self.system, 12)
        } else {
            Vec::new()
        };

        let mut snapshot = SystemSnapshot {
            taken_at_ms: now_ms(),
            cpu,
            memory,
            network,
            disk,
            // No cheap, dependency-free GPU busy counter exists across our
            // targets (Windows needs PDH, macOS IOKit); the widget renders
            // None as an idle ring, matching fnOS on headless machines.
            gpu_usage: None,
            uptime_secs: System::uptime(),
            processes: self.system.processes().len(),
            top_processes,
            volumes: Vec::new(),
            battery: None,
            host: self.host.clone(),
        };

        // Volumes and battery need syscalls / subprocesses that no UI needs at
        // 1 Hz, so they are refreshed on a slower cadence.
        let volumes_stale = self
            .volumes_at
            .map(|at| at.elapsed() > Duration::from_secs(5))
            .unwrap_or(true);
        if volumes_stale {
            let platform = squid_platform::current();
            if let Ok(volumes) = platform.volumes() {
                self.volumes = volumes;
            }
            self.volumes_at = Some(Instant::now());
        }
        snapshot.volumes = self.volumes.clone();

        let battery_stale = self
            .battery_at
            .map(|at| at.elapsed() > Duration::from_secs(20))
            .unwrap_or(true);
        if battery_stale {
            self.battery = Battery::current();
            self.battery_at = Some(Instant::now());
        }
        snapshot.battery = self.battery.clone();

        self.had_first_sample = true;
        snapshot
    }

    pub fn volumes(&mut self) -> Vec<Volume> {
        let stale = self
            .volumes_at
            .map(|at| at.elapsed() > Duration::from_secs(5))
            .unwrap_or(true);
        if stale {
            let platform = squid_platform::current();
            if let Ok(volumes) = platform.volumes() {
                self.volumes = volumes;
            }
            self.volumes_at = Some(Instant::now());
        }
        self.volumes.clone()
    }

    pub fn has_first_sample(&self) -> bool {
        self.had_first_sample
    }

    /// The row the panel would draw for one pid right now, or `None` once no
    /// process holds that number.
    ///
    /// Re-reading one pid is cheap, and it is the only way to find out whether a
    /// sample taken on an earlier beat still describes what is running: a row is
    /// a claim about a moment, not about a process, because pids are handed out
    /// again. [`Sampler::stop`] asks this question before it kills anything, and
    /// a test or an interface that wants one honest row asks it the same way.
    pub fn sample_of(&mut self, pid: u32) -> Option<ProcessSample> {
        let key = sysinfo::Pid::from_u32(pid);
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[key]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let process = self.system.process(key)?;
        Some(ProcessSample {
            pid,
            name: shown_name(process.name()),
            cpu: process.cpu_usage(),
            memory: Bytes(process.memory()),
        })
    }

    /// Force the process behind one sample to stop.
    ///
    /// The sample is what the user read; the pid inside it is what would be
    /// killed, and those two are not the same promise. Pids are recycled, so a
    /// list drawn a second ago can already be pointing a name at somebody
    /// else's process. The pid is therefore read again here, the rule from
    /// [`squid_platform::process::refusal`] is applied to the process that is
    /// really there, and a name that no longer matches is reported as *gone* —
    /// the thing that was pointed at is gone, whichever direction the pid moved.
    ///
    /// Nothing here asks the process politely. There is no TERM-then-KILL
    /// handshake, because the interface says 强制结束 / force quit and a dialog
    /// that promises the bytes on disk are the only thing kept.
    pub fn stop(&mut self, target: &ProcessSample) -> Result<(), StopError> {
        if let Some(rule) = squid_platform::process::pid_refusal(target.pid) {
            return Err(StopError::Refused(rule));
        }
        let Some(current) = self.sample_of(target.pid) else {
            return Err(StopError::Gone { pid: target.pid });
        };
        if let Some(rule) = squid_platform::process::refusal(target.pid, &current.name) {
            return Err(StopError::Refused(rule));
        }
        if current.name != target.name {
            return Err(StopError::Gone { pid: target.pid });
        }
        squid_platform::process::force_stop(target.pid)
    }

    /// Per-application CPU totals. iStat Menus shows this next to the gauges.
    pub fn applications(&mut self, limit: usize) -> Vec<AppUsage> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let mut merged: std::collections::HashMap<String, (f32, u64, usize, u32)> =
            std::collections::HashMap::new();
        for process in self.system.processes().values() {
            let name = shown_name(process.name());
            let entry = merged
                .entry(name)
                .or_insert((0.0, 0, 0, process.pid().as_u32()));
            entry.0 += process.cpu_usage();
            entry.1 += process.memory();
            entry.2 += 1;
        }
        let mut apps: Vec<AppUsage> = merged
            .into_iter()
            .map(|(name, (cpu, memory, instances, pid))| AppUsage {
                name,
                cpu: cpu.min(4_000.0),
                memory: Bytes(memory),
                instances,
                pid,
            })
            .collect();
        apps.sort_by(|a, b| {
            b.cpu
                .partial_cmp(&a.cpu)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.memory.cmp(&a.memory))
        });
        apps.truncate(limit);
        apps
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppUsage {
    pub name: String,
    pub cpu: f32,
    pub memory: Bytes,
    pub instances: usize,
    pub pid: u32,
}

fn cpu_stats(system: &System) -> CpuStats {
    // sysinfo ≥ 0.34 keeps the aggregate out of `cpus()`: every entry is one
    // logical core, and the machine-wide number lives in `global_cpu_usage()`.
    let cores = system.cpus();
    let per_core: Vec<f32> = cores
        .iter()
        .map(|cpu| cpu.cpu_usage().clamp(0.0, 100.0))
        .collect();
    let load = System::load_average();
    let brand = cores
        .first()
        .map(|cpu| cpu.brand().trim().to_string())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let base_frequency_ghz = cores
        .first()
        .map(|cpu| cpu.frequency() as f64 / 1_000.0)
        .unwrap_or(0.0);
    let total = system.global_cpu_usage().clamp(0.0, 100.0);
    // sysinfo can miss a logical core on some Windows boards; the thread
    // count is the number the OS itself schedules against.
    let scheduled = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(0);
    let logical_cores = per_core.len().max(scheduled).max(1);
    let per_core = if per_core.len() < logical_cores && !per_core.is_empty() {
        let mean = total;
        let mut padded = per_core;
        padded.resize(logical_cores, mean);
        padded
    } else {
        per_core
    };
    let physical = System::physical_core_count().or_else(detect_physical_cores);
    let physical = physical.unwrap_or_else(|| logical_cores.div_ceil(2));
    CpuStats {
        total,
        per_core,
        load: [load.one as f32, load.five as f32, load.fifteen as f32],
        physical_cores: physical,
        logical_cores,
        brand,
        base_frequency_ghz,
    }
}

fn detect_physical_cores() -> Option<usize> {
    #[cfg(target_os = "macos")]
    {
        let value = sysctl_u64("hw.physicalcpu")?;
        Some(value as usize)
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
        let mut ids = std::collections::HashSet::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("core id") {
                if let Some(value) = rest.split(':').nth(1) {
                    ids.insert(value.trim().to_string());
                }
            }
        }
        if !ids.is_empty() {
            return Some(ids.len());
        }
        return Some(
            usize::from_str(
                std::fs::read_to_string("/sys/devices/system/cpu/present")
                    .ok()?
                    .trim(),
            )
            .unwrap_or(1),
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Option<u64> {
    use std::ffi::CString;
    use std::mem::{size_of, MaybeUninit};
    let c_name = CString::new(name).ok()?;
    let mut value: u64 = 0;
    let mut len = size_of::<u64>();
    let rc = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            &mut value as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 {
        Some(value)
    } else {
        let _ = MaybeUninit::<u64>::uninit();
        None
    }
}

fn memory_stats(system: &System) -> MemoryStats {
    let total = Bytes(system.total_memory());
    let installed = installed_memory().max(total);
    let mut stats = MemoryStats {
        installed,
        total,
        used: Bytes(system.used_memory()),
        available: Bytes(system.available_memory()),
        swap_total: Bytes(system.total_swap()),
        swap_used: Bytes(system.used_swap()),
        compressed: None,
        wired: None,
    };
    if let Some(extra) = vm_stat_details() {
        stats.compressed = Some(extra.0);
        stats.wired = Some(extra.1);
    }
    stats
}

/// RAM the machine physically ships with. Windows reports this *including*
/// what the firmware and the integrated GPU carve out of `total_memory()`;
/// macOS reads the same number from sysctl. Unknown platforms fall back to
/// the usable total so the installed−reserved split degrades to zero.
fn installed_memory() -> Bytes {
    #[cfg(target_os = "windows")]
    {
        let mut kb: u64 = 0;
        // SAFETY: plain out-parameter call; the API only writes the u64.
        let ok = unsafe {
            windows_sys::Win32::System::SystemInformation::GetPhysicallyInstalledSystemMemory(
                &mut kb,
            )
        };
        if ok == 0 {
            Bytes(0)
        } else {
            Bytes(kb.saturating_mul(1024))
        }
    }
    #[cfg(target_os = "macos")]
    {
        sysctl_u64("hw.memsize").map(Bytes).unwrap_or_default()
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/meminfo") {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    if let Some(kb) = rest
                        .trim()
                        .trim_end_matches("kB")
                        .trim()
                        .parse::<u64>()
                        .ok()
                    {
                        return Bytes(kb * 1024);
                    }
                }
            }
        }
        Bytes(0)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Bytes(0)
    }
}

/// `vm_stat` gives compressor/wired sizes that sysinfo folds away. Parsed
/// lazily and cheaply (one subprocess every few seconds at most).
fn vm_stat_details() -> Option<(Bytes, Bytes)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = now_ms();
    let previous = LAST.swap(now, Ordering::Relaxed);
    static CACHE: std::sync::Mutex<Option<(Bytes, Bytes)>> = std::sync::Mutex::new(None);
    if now.saturating_sub(previous) > 5_000 || previous == 0 {
        let output = std::process::Command::new("/usr/bin/vm_stat")
            .output()
            .ok()
            .filter(|out| out.status.success())?;
        let text = String::from_utf8_lossy(&output.stdout);
        let value = parse_vm_stat(&text)?;
        if let Ok(mut guard) = CACHE.lock() {
            *guard = Some(value);
        }
        Some(value)
    } else {
        CACHE.lock().ok().and_then(|guard| *guard)
    }
}

/// Apple Silicon uses 16 KiB pages, Intel Macs use 4 KiB. The header states the
/// size, and the two fields squid reports use different labels than older
/// versions of `vm_stat`; both facts made the previous parser return zero.
fn parse_vm_stat(text: &str) -> Option<(Bytes, Bytes)> {
    let header = text.lines().next()?;
    let (_, after_size) = header.split_once("page size of ")?;
    let page_size: u64 = after_size
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    let mut compressor_pages = 0u64;
    let mut wired_pages = 0u64;
    for line in text.lines().skip(1) {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let pages: u64 = value
            .trim()
            .trim_end_matches('.')
            .replace(' ', "")
            .parse()
            .unwrap_or(0);
        match key.trim().trim_matches('"') {
            "Pages occupied by compressor" => compressor_pages = pages,
            "Pages wired down" => wired_pages = pages,
            _ => {}
        }
    }
    Some((
        Bytes(compressor_pages.saturating_mul(page_size)),
        Bytes(wired_pages.saturating_mul(page_size)),
    ))
}

fn net_rates(networks: &Networks, seconds: f64) -> NetRate {
    let mut rx = 0u64;
    let mut tx = 0u64;
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    let mut best: Option<(u64, String)> = None;
    let mut interfaces = 0usize;
    for (name, data) in networks.list() {
        if name == "lo" || name.starts_with("lo") || name.starts_with("utun") {
            // Loopback and VPN tunnels would inflate the totals with traffic
            // that never leaves the machine.
            continue;
        }
        interfaces += 1;
        let delta = data.received() + data.transmitted();
        rx += data.received();
        tx += data.transmitted();
        rx_total += data.total_received();
        tx_total += data.total_transmitted();
        if best.as_ref().map(|(seen, _)| delta > *seen).unwrap_or(true) {
            best = Some((delta, name.clone()));
        }
    }
    NetRate {
        rx_per_sec: Bytes((rx as f64 / seconds) as u64),
        tx_per_sec: Bytes((tx as f64 / seconds) as u64),
        rx_total: Bytes(rx_total),
        tx_total: Bytes(tx_total),
        interfaces,
        primary: best.map(|(_, name)| name),
    }
}

fn disk_rates(disks: &Disks, seconds: f64) -> DiskRate {
    let mut read = 0u64;
    let mut write = 0u64;
    let mut physical = 0usize;
    for disk in disks.list() {
        if disk.is_removable() && disk.name().is_empty() {
            continue;
        }
        let usage: DiskUsage = disk.usage();
        read += usage.read_bytes;
        write += usage.written_bytes;
        physical += 1;
    }
    // sysinfo exposes no busy time, so approximate activity by throughput
    // against a conservative 1 GB/s ceiling for internal NVMe.
    let peak = ((read + write) as f64 / seconds).max(1.0);
    DiskRate {
        read_per_sec: Bytes((read as f64 / seconds) as u64),
        write_per_sec: Bytes((write as f64 / seconds) as u64),
        activity: ((peak / 1_000_000_000.0) * 100.0).min(100.0) as f32,
        disks: physical,
    }
}

/// The name a process is listed by: the last component of whatever the OS
/// handed over, which is a path on macOS and already a bare name on Windows.
///
/// One function because the name is also the proof. A sample is a snapshot of a
/// pid at one moment, and pids are recycled; before squid stops anything it
/// re-reads the pid and compares, which only means something if both sides
/// spell the name the same way.
fn shown_name(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy()
        .split(['/', '\\'])
        .next_back()
        .filter(|part| !part.is_empty())
        .unwrap_or("?")
        .to_string()
}

fn top_processes(system: &System, limit: usize) -> Vec<ProcessSample> {
    let mut processes: Vec<ProcessSample> = system
        .processes()
        .values()
        .map(|process| ProcessSample {
            pid: process.pid().as_u32(),
            name: shown_name(process.name()),
            cpu: process.cpu_usage(),
            memory: Bytes(process.memory()),
        })
        .collect();
    processes.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.memory.cmp(&a.memory))
    });
    processes.truncate(limit);
    processes
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

// ─────────────────────────────── history ──────────────────────────────────

/// Fixed-capacity ring buffer used for the sparklines in the TUI.
#[derive(Clone, Debug)]
pub struct History<T> {
    cap: usize,
    items: VecDeque<T>,
}

impl<T: Clone> History<T> {
    pub fn new(cap: usize) -> History<T> {
        History {
            cap: cap.max(2),
            items: VecDeque::with_capacity(cap.max(2)),
        }
    }

    pub fn push(&mut self, value: T) {
        if self.items.len() >= self.cap {
            self.items.pop_front();
        }
        self.items.push_back(value);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn values(&self) -> Vec<T> {
        self.items.iter().cloned().collect()
    }

    pub fn last(&self) -> Option<&T> {
        self.items.back()
    }

    pub fn oldest(&self) -> Option<&T> {
        self.items.front()
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// Snapshot history for the state screen: CPU, memory pressure and net rates.
#[derive(Clone, Debug)]
pub struct TrendStore {
    pub cpu: History<f32>,
    pub memory: History<f32>,
    pub rx: History<f64>,
    pub tx: History<f64>,
    pub read: History<f64>,
    pub write: History<f64>,
    pub uptime_secs: History<u64>,
}

impl Default for TrendStore {
    fn default() -> Self {
        TrendStore {
            cpu: History::new(120),
            memory: History::new(120),
            rx: History::new(120),
            tx: History::new(120),
            read: History::new(120),
            write: History::new(120),
            uptime_secs: History::new(120),
        }
    }
}

impl TrendStore {
    pub fn record(&mut self, snapshot: &SystemSnapshot) {
        self.cpu.push(snapshot.cpu.total);
        self.memory.push(snapshot.memory.percent_used() as f32);
        self.rx.push(snapshot.network.rx_per_sec.as_u64() as f64);
        self.tx.push(snapshot.network.tx_per_sec.as_u64() as f64);
        self.read.push(snapshot.disk.read_per_sec.as_u64() as f64);
        self.write.push(snapshot.disk.write_per_sec.as_u64() as f64);
        self.uptime_secs.push(snapshot.uptime_secs);
    }

    pub fn trimmed(&self, width: usize) -> TrendWindow {
        fn tail<T: Clone>(history: &History<T>, width: usize) -> Vec<T> {
            let values = history.values();
            let skip = values.len().saturating_sub(width.max(1));
            values[skip..].to_vec()
        }
        TrendWindow {
            cpu: tail(&self.cpu, width),
            memory: tail(&self.memory, width),
            rx: tail(&self.rx, width),
            tx: tail(&self.tx, width),
            read: tail(&self.read, width),
            write: tail(&self.write, width),
        }
    }
}

/// Sparkline-ready view of the last `width` samples.
#[derive(Clone, Debug, Default)]
pub struct TrendWindow {
    pub cpu: Vec<f32>,
    pub memory: Vec<f32>,
    pub rx: Vec<f64>,
    pub tx: Vec<f64>,
    pub read: Vec<f64>,
    pub write: Vec<f64>,
}

pub fn path_of(volume: &Volume) -> PathBuf {
    volume.mount_point.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formatting() {
        assert_eq!(human_uptime(59), "0m 59s");
        assert_eq!(human_uptime(3_700), "1h 1m");
        assert_eq!(human_uptime(90_000), "1d 1h 0m");
    }

    #[test]
    fn history_is_a_ring_buffer() {
        let mut history = History::new(3);
        for value in 0..5 {
            history.push(value);
        }
        assert_eq!(history.values(), vec![2, 3, 4]);
        assert_eq!(history.last(), Some(&4));
        assert_eq!(history.oldest(), Some(&2));
    }

    #[test]
    fn sampler_produces_sane_numbers() {
        let mut sampler = Sampler::new();
        std::thread::sleep(Duration::from_millis(220));
        let snapshot = sampler.refresh();
        assert!(snapshot.cpu.logical_cores > 0);
        assert_eq!(snapshot.cpu.per_core.len(), snapshot.cpu.logical_cores);
        assert!(snapshot.memory.total.as_u64() > 0);
        assert!(snapshot.memory.percent_used() <= 100);
        assert!(!snapshot.volumes.is_empty(), "expected at least one volume");
        assert!(snapshot.taken_at_ms > 0);
        assert!(!snapshot.host.os_name.is_empty());
        assert!(snapshot.processes > 0);
        let apps = sampler.applications(5);
        assert!(!apps.is_empty());
    }

    /// The widget-grade light refresh: every meter the desktop panel draws
    /// still moves, but the per-process table stays empty (that walk is the
    /// expensive part of a refresh on Windows).
    #[test]
    fn the_light_refresh_keeps_the_meters_and_drops_the_table() {
        let mut sampler = Sampler::new();
        std::thread::sleep(Duration::from_millis(220));
        let snapshot = sampler.refresh_with(false);
        assert_eq!(snapshot.cpu.per_core.len(), snapshot.cpu.logical_cores);
        assert!(snapshot.memory.total.as_u64() > 0);
        assert!(snapshot.processes > 0);
        assert!(snapshot.top_processes.is_empty());
        // A light beat must not poison the full one: the next full refresh
        // still walks processes and produces a ranking.
        let full = sampler.refresh_with(true);
        assert!(!full.top_processes.is_empty() || full.processes == 0);
    }

    #[test]
    fn percentages_never_exceed_one_hundred() {
        let stats = MemoryStats {
            total: Bytes::new(100),
            used: Bytes::new(1_000),
            ..Default::default()
        };
        assert_eq!(stats.percent_used(), 100);
    }

    #[test]
    fn vm_stat_uses_the_reported_page_size_and_fields() {
        let text = "\
Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
Pages free:                               96476.\n\
Pages wired down:                       122963.\n\
Pages occupied by compressor:           151882.\n";
        let (compressed, wired) = parse_vm_stat(text).expect("vm_stat");
        assert_eq!(wired, Bytes(122_963 * 16_384));
        assert_eq!(compressed, Bytes(151_882 * 16_384));
    }

    /// The feature, end to end: a sample that still describes its pid stops it.
    #[test]
    fn a_process_stops_when_its_sample_asks_it_to() {
        let mut sampler = Sampler::new();
        let mut child = sleeper();
        let sample = sample_of(&mut sampler, child.id());
        assert!(
            child.try_wait().expect("child is running").is_none(),
            "the sleeper died on its own: {}",
            sample.name
        );
        sampler
            .stop(&sample)
            .expect("stopping a child of this test must be allowed");
        let status = child.wait().expect("the child is reaped");
        assert!(!status.success(), "a stopped process exited cleanly");
    }

    /// The guard, end to end: a pid that no longer belongs to the name the user
    /// read stops nothing at all.
    ///
    /// Pids are recycled, so this is not a hypothetical race — a list is drawn
    /// when the panel opens and read from seconds later, which is long enough
    /// for an app to close and a new one to be handed its number. Killing on the
    /// number alone would punish whoever inherited it.
    #[test]
    fn a_sample_that_no_longer_describes_its_pid_stops_nothing() {
        let mut sampler = Sampler::new();
        let mut child = sleeper();
        let pid = child.id();
        let mut sample = sample_of(&mut sampler, pid);
        sample.name = "WhateverWasOnScreen".into();
        let error = sampler
            .stop(&sample)
            .expect_err("the name does not belong to this pid");
        assert!(
            matches!(error, StopError::Gone { pid: gone } if gone == pid),
            "expected Gone({pid}), got {error:?}"
        );
        assert!(
            child.try_wait().expect("child is still running").is_none(),
            "the process that inherited the pid was punished for it"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    /// The rules are checked against the process that is really there, which is
    /// why this one needs no name to be right: pid zero answers to the kernel on
    /// unix and to the System Idle Process on Windows, and neither is a target.
    #[test]
    fn squid_leaves_the_untouchable_processes_alone() {
        let mut sampler = Sampler::new();
        let myself = ProcessSample {
            pid: std::process::id(),
            name: "whatever the list said".into(),
            cpu: 0.0,
            memory: Bytes(0),
        };
        assert_eq!(
            sampler.stop(&myself).expect_err("this is the test itself"),
            StopError::Refused(squid_platform::process::OffLimits::Itself)
        );
        let first = ProcessSample { pid: 0, ..myself };
        assert_eq!(
            sampler.stop(&first).expect_err("pid zero"),
            StopError::Refused(squid_platform::process::OffLimits::First)
        );
    }

    /// A process long enough to be seen, harmless enough to lose.
    fn sleeper() -> std::process::Child {
        let mut command = if cfg!(windows) {
            let mut command = std::process::Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        } else {
            let mut command = std::process::Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("a loopback ping / sleep should start")
    }

    /// The row as the panel would have drawn it, name included: read from
    /// the operating system rather than guessed, because the name is the proof
    /// `stop` checks and a guessed name would fail for the wrong reason.
    fn sample_of(sampler: &mut Sampler, pid: u32) -> ProcessSample {
        sampler
            .sample_of(pid)
            .expect("a child of this test is a process")
    }
}
