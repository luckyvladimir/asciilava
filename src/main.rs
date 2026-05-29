use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent},
    execute,
    terminal,
};
use rand::Rng;
use std::io::{self, Write};
use std::time::{Duration, Instant};

const FPS: u64 = 30;
const FRAME_DUR: Duration = Duration::from_millis(1000 / FPS);

// Standard 16 terminal colors as ANSI codes (foreground)
// (core, glow, dim)
const COLOR_SLOTS: &[(u8, u8, u8)] = &[
    (91, 93, 33),  // bright red / bright yellow / dark yellow
    (92, 96, 36),  // bright green / bright cyan / dark cyan
    (94, 96, 36),  // bright blue / bright cyan / dark cyan
    (95, 91, 31),  // bright magenta / bright red / dark red
    (93, 97, 37),  // bright yellow / bright white / white
    (96, 92, 32),  // bright cyan / bright green / dark green
];

struct Blob {
    x: f64,
    y: f64,
    radius: f64,
    base_radius: f64,
    vx: f64,
    vy: f64,
    phase: f64,
    pulse_speed: f64,
    wobble_phase: f64,
    wobble_speed: f64,
    age: u64,
    temperature: f64,
}

impl Blob {
    fn new(x: f64, y: f64, radius: f64, height: f64) -> Self {
        let mut rng = rand::thread_rng();
        let t = y / height;
        Self {
            x, y,
            radius,
            base_radius: radius,
            vx: 0.0,
            vy: 0.0,
            phase: rng.gen::<f64>() * std::f64::consts::TAU,
            pulse_speed: rng.gen_range(0.02..0.06),
            wobble_phase: rng.gen::<f64>() * std::f64::consts::TAU,
            wobble_speed: rng.gen_range(0.01..0.04),
            age: 0,
            temperature: if t > 0.7 { rng.gen_range(0.7..1.0) } else { rng.gen_range(0.0..0.4) },
        }
    }

    fn update(&mut self, width: u16, height: u16, others: &[(f64, f64, f64)]) {
        self.age += 1;
        let h = height as f64;
        let w = width as f64;
        let norm_y = self.y / h;

        if norm_y > 0.8 {
            self.temperature += 0.003;
        } else if norm_y < 0.2 {
            self.temperature -= 0.002;
        } else {
            self.temperature -= 0.0004;
        }
        self.temperature = self.temperature.clamp(0.0, 1.0);

        let buoyancy = (self.temperature - 0.5) * -0.01;
        self.vy += buoyancy;

        self.vx += (self.age as f64 * 0.002 + self.wobble_phase).sin() * 0.0005;

        self.vx *= 0.95;
        self.vy *= 0.985;

        self.vx = self.vx.clamp(-0.05, 0.05);
        self.vy = self.vy.clamp(-0.12, 0.12);

        self.x += self.vx;
        self.y += self.vy;

        if self.x - self.radius < 0.0 {
            self.x = self.radius;
            self.vx = self.vx.abs() * 0.3;
        }
        if self.x + self.radius > w - 1.0 {
            self.x = w - 1.0 - self.radius;
            self.vx = -(self.vx.abs()) * 0.3;
        }
        if self.y - self.radius < 0.0 {
            self.y = self.radius;
            self.vy = self.vy.abs() * 0.2;
            self.temperature -= 0.004;
        }
        if self.y + self.radius > h - 1.0 {
            self.y = h - 1.0 - self.radius;
            self.vy = -(self.vy.abs()) * 0.2;
            self.temperature += 0.004;
        }

        self.phase += self.pulse_speed;
        self.wobble_phase += self.wobble_speed;
        self.radius = self.base_radius + self.phase.sin() * (self.base_radius * 0.3);

        for &(ox, oy, or) in others {
            let dx = self.x - ox;
            let dy = self.y - oy;
            let dist = (dx * dx + dy * dy).sqrt() + 0.01;
            let min_dist = self.radius + or;
            if dist < min_dist * 1.5 {
                let force = (min_dist * 1.5 - dist) * 0.008;
                self.vx += (dx / dist) * force * 1.5;
                self.vy += (dy / dist) * force * 0.3;
            }
        }
    }
}

/// Interpolate between two color slots based on t (0.0 = slot a, 1.0 = slot b).
/// Since we can't blend ANSI codes, we crossfade by picking the closer one,
/// but use `t` to decide which tier (core/glow/dim) maps to which slot.
fn blend_colors(a: (u8, u8, u8), b: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    if t < 0.33 {
        // Mostly A
        (a.0, a.1, a.2)
    } else if t < 0.5 {
        // Transition: B's core starts bleeding into A's glow
        (a.0, b.1, a.2)
    } else if t < 0.66 {
        // Transition: A's dim fades, B takes over
        (b.0, a.1, b.2)
    } else {
        // Mostly B
        (b.0, b.1, b.2)
    }
}

struct LavaLamp {
    width: u16,
    height: u16,
    blobs: Vec<Blob>,
    frame: u64,
    color_phase: f64,
    last_color_change: Instant,
    blob_count: usize,
}

impl LavaLamp {
    fn new() -> io::Result<Self> {
        let (width, height) = terminal::size()?;
        let mut rng = rand::thread_rng();
        let w = width as f64;
        let h = height as f64;
        let initial_count = ((w * h) / 400.0).clamp(3.0, 5.0) as usize;
        let mut lamp = Self {
            width, height,
            blobs: Vec::new(),
            frame: 0,
            color_phase: rng.gen_range(0.0..COLOR_SLOTS.len() as f64),
            last_color_change: Instant::now(),
            blob_count: initial_count,
        };
        lamp.spawn_blobs();
        Ok(lamp)
    }

    fn current_colors(&self) -> (u8, u8, u8) {
        let idx_a = (self.color_phase.floor() as usize) % COLOR_SLOTS.len();
        let idx_b = (idx_a + 1) % COLOR_SLOTS.len();
        let t = self.color_phase - self.color_phase.floor();
        blend_colors(COLOR_SLOTS[idx_a], COLOR_SLOTS[idx_b], t)
    }

    fn spawn_blobs(&mut self) {
        self.blobs.clear();
        let mut rng = rand::thread_rng();
        let w = self.width as f64;
        let h = self.height as f64;

        for i in 0..self.blob_count {
            let max_r = (w / 10.0).max(2.0).min(6.0);
            let r = rng.gen_range(1.5..=max_r);
            let x = rng.gen_range(r..w - r);
            let y = if i % 2 == 0 {
                rng.gen_range(h * 0.7..h - r)
            } else {
                rng.gen_range(r..h * 0.3)
            };
            self.blobs.push(Blob::new(x, y, r, h));
        }
    }

    fn random_color(&mut self) {
        let mut rng = rand::thread_rng();
        self.color_phase = rng.gen_range(0.0..COLOR_SLOTS.len() as f64);
        self.last_color_change = Instant::now();
    }

    fn add_blob(&mut self) {
        if self.blob_count >= 100 { return; }
        self.blob_count += 1;
        let mut rng = rand::thread_rng();
        let w = self.width as f64;
        let h = self.height as f64;
        let max_r = (w / 10.0).max(2.0).min(6.0);
        let r = rng.gen_range(1.5..=max_r);
        let x = rng.gen_range(r..w - r);
        // New blob starts at bottom, hot
        let y = rng.gen_range(h * 0.7..h - r);
        self.blobs.push(Blob::new(x, y, r, h));
    }

    fn remove_blob(&mut self) {
        if self.blob_count <= 1 { return; }
        self.blob_count -= 1;
        if !self.blobs.is_empty() {
            self.blobs.pop();
        }
    }

    fn metaball_field(&self, px: f64, py: f64) -> f64 {
        let mut total = 0.0;
        for blob in &self.blobs {
            let dx = px - blob.x;
            let dy = (py - blob.y) * 2.0;
            let dist_sq = dx * dx + dy * dy + 0.1;
            total += (blob.radius * blob.radius) / dist_sq;
        }
        total
    }

    fn update(&mut self) {
        let positions: Vec<(f64, f64, f64)> = self.blobs.iter()
            .map(|b| (b.x, b.y, b.radius))
            .collect();

        for (i, blob) in self.blobs.iter_mut().enumerate() {
            let others: Vec<_> = positions.iter().enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, p)| *p)
                .collect();
            blob.update(self.width, self.height, &others);
        }

        // Auto color change every 60 seconds
        if self.last_color_change.elapsed() >= Duration::from_secs(60) {
            self.random_color();
        }

        self.frame += 1;
    }

    fn render(&self, buf: &mut Vec<u8>) -> io::Result<()> {
        let w = self.width;
        let h = self.height;
        let colors = self.current_colors();

        buf.extend_from_slice(b"\x1b[2J\x1b[H");

        if h < 4 || w < 8 {
            buf.extend_from_slice(b"Terminal too small");
            return Ok(());
        }

        let mut last_color: u8 = 0;

        for y in 0..h {
            for x in 0..w {
                let field = self.metaball_field(x as f64, y as f64);
                if field < 0.12 { continue; }

                let (ch, color, bold) = if field > 1.5 {
                    (b'@', colors.0, true)
                } else if field > 1.0 {
                    (b'#', colors.0, true)
                } else if field > 0.7 {
                    (b'%', colors.0, false)
                } else if field > 0.5 {
                    (b'*', colors.1, true)
                } else if field > 0.35 {
                    (b'+', colors.1, false)
                } else if field > 0.25 {
                    (b':', colors.2, false)
                } else if field > 0.18 {
                    (b'.', colors.2, false)
                } else {
                    continue;
                };

                move_to(buf, y, x);
                if color != last_color || bold {
                    set_fg(buf, color, bold);
                    last_color = color;
                }
                buf.push(ch);
            }
        }

        buf.extend_from_slice(b"\x1b[0m");
        Ok(())
    }

    fn run(&mut self) -> io::Result<()> {
        let mut stdout = io::stdout();
        let mut buf: Vec<u8> = Vec::with_capacity(128 * 1024);

        loop {
            let frame_start = Instant::now();

            while event::poll(Duration::ZERO)? {
                match event::read()? {
                    Event::Key(KeyEvent { code, modifiers, .. }) => match code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('c') if modifiers.contains(event::KeyModifiers::CONTROL) => return Ok(()),
                        KeyCode::Char('c') => self.random_color(),
                        KeyCode::Char('r') => self.spawn_blobs(),
                        KeyCode::Char('+') | KeyCode::Char('=') => self.add_blob(),
                        KeyCode::Char('-') | KeyCode::Char('_') => self.remove_blob(),
                        _ => {}
                    },
                    Event::Resize(w, h) => {
                        self.width = w;
                        self.height = h;
                        self.spawn_blobs();
                    }
                    _ => {}
                }
            }

            self.update();

            buf.clear();
            self.render(&mut buf)?;
            stdout.write_all(&buf)?;
            stdout.flush()?;

            let elapsed = frame_start.elapsed();
            if elapsed < FRAME_DUR {
                std::thread::sleep(FRAME_DUR - elapsed);
            }
        }
    }
}

fn move_to(buf: &mut Vec<u8>, row: u16, col: u16) {
    use io::Write as _;
    write!(buf, "\x1b[{};{}H", row + 1, col + 1).unwrap();
}

fn set_fg(buf: &mut Vec<u8>, code: u8, bold: bool) {
    use io::Write as _;
    if bold {
        write!(buf, "\x1b[1;{}m", code).unwrap();
    } else {
        write!(buf, "\x1b[0;{}m", code).unwrap();
    }
}

fn main() -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = LavaLamp::new()?.run();

    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}
