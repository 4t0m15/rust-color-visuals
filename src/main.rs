use anyhow::Result;
use glam::Vec2;
use image::{ImageBuffer, Rgba};
use noise::{NoiseFn, Perlin};
use pixels::{Pixels, SurfaceTexture};
use rand::{rngs::StdRng, Rng, SeedableRng};
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Event, KeyboardInput, VirtualKeyCode, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

const WIDTH: u32 = 800;
const HEIGHT: u32 = 800;

#[derive(Clone, Copy)]
struct Particle {
    pos: Vec2,
    vel: Vec2,
    age: u32,
    alive: bool,
}

impl Particle {
    fn new(pos: Vec2) -> Self {
        Self {
            pos,
            vel: Vec2::ZERO,
            age: 0,
            alive: true,
        }
    }
}

#[derive(Clone, Copy)]
enum ColorMode {
    Direction,
    Age,
    Curl,
}

fn hsv_to_rgb(mut h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    h = h.fract();
    if h < 0.0 {
        h += 1.0;
    }
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (
        (r * 255.0).clamp(0.0, 255.0) as u8,
        (g * 255.0).clamp(0.0, 255.0) as u8,
        (b * 255.0).clamp(0.0, 255.0) as u8,
    )
}

struct Params {
    scale: f32,
    z: f32,
    z_step: f32,
    force: f32,
    friction: f32,
    steps_per_frame: usize,
    spawn_count: usize,
    fade: f32,
    color_mode: ColorMode,
    paused: bool,
}

struct App {
    width: u32,
    height: u32,
    pixels: Pixels,
    perlin: Perlin,
    noise_seed: u32,
    rng: StdRng,
    params: Params,
    particles: Vec<Particle>,
    frame_index: u64,
}

impl App {
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        if let Err(e) = self.pixels.resize_buffer(width, height) {
            eprintln!("pixels buffer resize failed: {}", e);
            return;
        }
        // Clear the newly sized frame to fully opaque black so no stale data shows
        let frame = self.pixels.frame_mut();
        for px in frame.chunks_exact_mut(4) {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 255;
        }
    }
    fn new(mut pixels: Pixels, width: u32, height: u32) -> Self {
        // Initialize frame to black and opaque alpha
        {
            let frame = pixels.frame_mut();
            for px in frame.chunks_exact_mut(4) {
                px[0] = 0;
                px[1] = 0;
                px[2] = 0;
                px[3] = 255;
            }
        }

        let noise_seed = 42u32;
        let perlin = Perlin::new(noise_seed);
        let rng = StdRng::seed_from_u64(123456789);

        let params = Params {
            scale: 0.004,
            z: 0.0,
            z_step: 0.004,
            force: 0.8,
            friction: 0.985,
            steps_per_frame: 300,
            spawn_count: (height / 4) as usize,
            fade: 0.03,
            color_mode: ColorMode::Direction,
            paused: false,
        };

        Self {
            width,
            height,
            pixels,
            perlin,
            noise_seed,
            rng,
            params,
            particles: Vec::with_capacity((width * height / 4) as usize),
            frame_index: 0,
        }
    }
}

impl App {
    fn handle_key(&mut self, input: KeyboardInput) {
        if input.state != ElementState::Pressed {
            return;
        }
        if let Some(key) = input.virtual_keycode {
            match key {
                VirtualKeyCode::Space => {
                    self.params.paused = !self.params.paused;
                }
                VirtualKeyCode::S => {
                    let _ = self.save_png();
                }
                VirtualKeyCode::R => self.reseed_noise(),
                VirtualKeyCode::LBracket => {
                    self.params.scale = (self.params.scale * 0.9).max(0.0005)
                }
                VirtualKeyCode::RBracket => {
                    self.params.scale = (self.params.scale * 1.111).min(0.05)
                }
                VirtualKeyCode::Comma => {
                    self.params.z_step = (self.params.z_step * 0.9).max(0.0001)
                }
                VirtualKeyCode::Period => {
                    self.params.z_step = (self.params.z_step * 1.111).min(0.05)
                }
                VirtualKeyCode::Slash => self.params.force = (self.params.force * 0.9).max(0.05),
                VirtualKeyCode::Equals => self.params.force = (self.params.force * 1.111).min(5.0),
                VirtualKeyCode::Key9 => {
                    self.params.friction = (self.params.friction - 0.002).max(0.90)
                }
                VirtualKeyCode::Key0 => {
                    self.params.friction = (self.params.friction + 0.002).min(0.9995)
                }
                VirtualKeyCode::F => self.params.fade = (self.params.fade + 0.01).min(0.2),
                VirtualKeyCode::G => self.params.fade = (self.params.fade - 0.01).max(0.0),
                VirtualKeyCode::C => self.cycle_color_mode(),
                _ => {}
            }
        }
    }

    fn cycle_color_mode(&mut self) {
        self.params.color_mode = match self.params.color_mode {
            ColorMode::Direction => ColorMode::Age,
            ColorMode::Age => ColorMode::Curl,
            ColorMode::Curl => ColorMode::Direction,
        };
    }

    fn reseed_noise(&mut self) {
        let seed: u32 = self.rng.gen();
        self.noise_seed = seed;
        self.perlin = Perlin::new(seed);
    }

    fn save_png(&mut self) -> anyhow::Result<()> {
        let frame = self.pixels.frame();
        let mut data = frame.to_vec();
        for i in (0..data.len()).step_by(4) {
            data[i + 3] = 255;
        }
        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, data).expect("buffer dims");
        let filename = format!("frame_{:06}.png", self.frame_index);
        img.save(&filename)?;
        println!("Saved {}", filename);
        Ok(())
    }

    fn apply_fade(&mut self) {
        let fade_scale = 1.0 - self.params.fade;
        if fade_scale >= 1.0 {
            return;
        }
        let frame = self.pixels.frame_mut();
        for px in frame.chunks_exact_mut(4) {
            px[0] = ((px[0] as f32) * fade_scale) as u8;
            px[1] = ((px[1] as f32) * fade_scale) as u8;
            px[2] = ((px[2] as f32) * fade_scale) as u8;
            px[3] = 255;
        }
    }

    fn spawn_particles(&mut self) {
        let width_f = self.width as f32;
        let height_f = self.height as f32;
        let count = self.params.spawn_count;
        if count == 0 {
            return;
        }
        let mut spawned = 0usize;
        let mut i = 0usize;
        // Reuse dead particle slots first
        while spawned < count && i < self.particles.len() {
            if !self.particles[i].alive {
                let pos = Vec2::new(
                    self.rng.gen_range(0.0..width_f),
                    self.rng.gen_range(0.0..height_f),
                );
                self.particles[i] = Particle::new(pos);
                spawned += 1;
            }
            i += 1;
        }
        // Then append any remaining new particles
        while spawned < count {
            let pos = Vec2::new(
                self.rng.gen_range(0.0..width_f),
                self.rng.gen_range(0.0..height_f),
            );
            self.particles.push(Particle::new(pos));
            spawned += 1;
        }
    }

    fn step_particles(&mut self) {
        let margin = 10.0;
        let width_f = self.width as f32;
        let height_f = self.height as f32;

        for particle in &mut self.particles {
            if !particle.alive {
                continue;
            }
            for _ in 0..self.params.steps_per_frame {
                let prev = particle.pos;
                let dir = noise_dir(&self.perlin, self.params.scale, self.params.z, particle.pos);
                particle.vel += dir * self.params.force;
                particle.vel *= self.params.friction;
                particle.pos += particle.vel;
                particle.age = particle.age.saturating_add(1);

                // Determine color now (no frame borrow yet)
                let color = match self.params.color_mode {
                    ColorMode::Direction => {
                        let angle = particle.vel.y.atan2(particle.vel.x);
                        let mut hue = (angle / std::f32::consts::TAU).fract();
                        if hue < 0.0 {
                            hue += 1.0;
                        }
                        let speed = particle.vel.length();
                        let v = (speed * 0.5).clamp(0.1, 1.0);
                        hsv_to_rgb(hue + self.params.z * 0.5, 1.0, v)
                    }
                    ColorMode::Age => {
                        let hue = ((particle.age as f32) * 0.002 + self.params.z * 0.5).fract();
                        let v = (particle.vel.length() * 0.5).clamp(0.1, 1.0);
                        hsv_to_rgb(hue, 1.0, v)
                    }
                    ColorMode::Curl => {
                        let eps = 2.0;
                        let a0 = noise_angle(&self.perlin, self.params.scale, self.params.z, prev);
                        let a1 = noise_angle(
                            &self.perlin,
                            self.params.scale,
                            self.params.z,
                            prev + Vec2::new(eps, 0.0),
                        );
                        let mut da = a1 - a0;
                        while da > std::f32::consts::PI {
                            da -= std::f32::consts::TAU;
                        }
                        while da < -std::f32::consts::PI {
                            da += std::f32::consts::TAU;
                        }
                        let hue = (da.abs() / std::f32::consts::PI).clamp(0.0, 1.0);
                        let v = (particle.vel.length() * 0.6).clamp(0.2, 1.0);
                        hsv_to_rgb(hue, 1.0, v)
                    }
                };

                // Borrow frame only for drawing
                {
                    let frame = self.pixels.frame_mut();
                    draw_segment_additive(
                        frame,
                        self.width,
                        self.height,
                        prev,
                        particle.pos,
                        color,
                    );
                }

                if particle.pos.x < -margin
                    || particle.pos.x > width_f + margin
                    || particle.pos.y < -margin
                    || particle.pos.y > height_f + margin
                {
                    particle.alive = false;
                    break;
                }
            }
        }
    }

    fn update_and_render(&mut self) {
        // Fade globally
        self.apply_fade();

        // Update simulation
        if !self.params.paused {
            self.spawn_particles();
            self.step_particles();
            self.params.z += self.params.z_step;
        }

        if let Err(e) = self.pixels.render() {
            eprintln!("pixels.render() failed: {}", e);
        } else {
            self.frame_index += 1;
        }
    }
}

fn main() -> Result<()> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Perlin Flow Particles")
        .with_inner_size(LogicalSize::new(WIDTH as f64, HEIGHT as f64))
        .build(&event_loop)?;

    let size = window.inner_size();
    let surface_texture = SurfaceTexture::new(size.width, size.height, &window);
    let pixels = Pixels::new(size.width, size.height, surface_texture)?;
    let mut app = App::new(pixels, size.width, size.height);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => {
                    *control_flow = ControlFlow::Exit;
                }
                WindowEvent::KeyboardInput { input, .. } => {
                    app.handle_key(input);
                }
                WindowEvent::Resized(size) => {
                    if let Err(e) = app.pixels.resize_surface(size.width, size.height) {
                        eprintln!("pixels surface resize failed: {}", e);
                    }
                    if size.width > 0 && size.height > 0 {
                        app.resize(size.width, size.height);
                    }
                }
                WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
                    let size = *new_inner_size;
                    if let Err(e) = app.pixels.resize_surface(size.width, size.height) {
                        eprintln!("pixels surface resize failed: {}", e);
                    }
                    if size.width > 0 && size.height > 0 {
                        app.resize(size.width, size.height);
                    }
                }
                _ => {}
            },
            Event::MainEventsCleared => {
                window.request_redraw();
            }
            Event::RedrawRequested(_) => {
                app.update_and_render();
            }
            _ => {}
        }
    });
}

fn noise_dir(perlin: &Perlin, scale: f32, z: f32, p: Vec2) -> Vec2 {
    let n = perlin.get([(p.x * scale) as f64, (p.y * scale) as f64, z as f64]) as f32;
    let angle = n * std::f32::consts::TAU;
    Vec2::new(angle.cos(), angle.sin())
}

fn noise_angle(perlin: &Perlin, scale: f32, z: f32, p: Vec2) -> f32 {
    let n = perlin.get([(p.x * scale) as f64, (p.y * scale) as f64, z as f64]) as f32;
    n * std::f32::consts::TAU
}

fn draw_segment_additive(
    frame: &mut [u8],
    width: u32,
    height: u32,
    p0: Vec2,
    p1: Vec2,
    color: (u8, u8, u8),
) {
    let (r, g, b) = color;

    let mut x0 = p0.x as i32;
    let mut y0 = p0.y as i32;
    let x1 = p1.x as i32;
    let y1 = p1.y as i32;

    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && y0 >= 0 && (x0 as u32) < width && (y0 as u32) < height {
            let idx = (((y0 as u32) * width + (x0 as u32)) * 4) as usize;
            frame[idx] = frame[idx].saturating_add(r);
            frame[idx + 1] = frame[idx + 1].saturating_add(g);
            frame[idx + 2] = frame[idx + 2].saturating_add(b);
            frame[idx + 3] = 255;
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}
