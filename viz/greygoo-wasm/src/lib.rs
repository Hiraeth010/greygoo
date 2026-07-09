//! Browser-side wrapper: the real `sim_core::World`, exposed over a raw C ABI so
//! plain JS can drive it with no wasm-bindgen toolchain. The grid is rendered to
//! an RGBA buffer inside wasm memory; JS blits it to a canvas. Because this is
//! the same engine as Phase 1 and the on-chain sector, what you watch is the
//! actual evolution, not a re-implementation.

use sim_core::{Config, World, AFF, AGGR, METAB};

pub struct App {
    world: World,
    frame: Vec<u8>, // RGBA, w*h*4
    w: usize,
    h: usize,
}

/// Create a world. Seed is passed as two u32 halves (JS numbers are f64).
/// `uniform != 0` fills every cell with habitat (a teeming field) instead of
/// two sugarscape peaks.
#[no_mangle]
pub extern "C" fn init(
    seed_lo: u32,
    seed_hi: u32,
    width: u32,
    height: u32,
    init_agents: u32,
    uniform: u32,
) -> *mut App {
    let w = width as usize;
    let h = height as usize;
    let cfg = Config {
        width: w,
        height: h,
        init_agents: init_agents as usize,
        uniform: uniform != 0,
        ..Config::default()
    };
    let seed = ((seed_hi as u64) << 32) | seed_lo as u64;
    let world = World::new(cfg, seed);
    let frame = vec![0u8; w * h * 4];
    Box::into_raw(Box::new(App { world, frame, w, h }))
}

/// Advance the world `n` ticks.
#[no_mangle]
pub extern "C" fn step(app: *mut App, n: u32) {
    let app = unsafe { &mut *app };
    for _ in 0..n {
        app.world.step();
    }
}

#[inline]
fn strain_color(strain: u32) -> (u8, u8, u8) {
    // hash the lineage id to a vivid, well-separated colour
    let mut x = strain.wrapping_mul(0x9E37_79B1);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA77);
    x ^= x >> 13;
    let mut r = (x & 0xff) as u8;
    let mut g = ((x >> 8) & 0xff) as u8;
    let mut b = ((x >> 16) & 0xff) as u8;
    // lift toward the bright end so lineages glow on black
    let m = r.max(g).max(b);
    if m < 180 {
        let boost = 180 - m;
        r = r.saturating_add(boost);
        g = g.saturating_add(boost);
        b = b.saturating_add(boost);
    }
    (r, g, b)
}

/// Paint the grid into the RGBA buffer and return a pointer to it.
/// mode 0 = genome (R=affinity, G=efficiency, B=aggression),
/// mode 1 = lineage (colour per founding strain),
/// mode 2 = vitality (energy heat). Empty cells always glow teal by resource.
#[no_mangle]
pub extern "C" fn render(app: *mut App, mode: u32) -> *const u8 {
    let app = unsafe { &mut *app };
    let cap_max = app.world.cfg.cap_max.max(1) as u32;
    let max_e = app.world.cfg.max_energy.max(1);
    for i in 0..app.w * app.h {
        let o = i * 4;
        match &app.world.cells[i] {
            Some(a) => {
                let (r, g, b) = match mode {
                    1 => strain_color(a.strain),
                    2 => {
                        let t = (a.energy.max(0) * 255 / max_e).min(255) as u8;
                        (t / 3, t, (t / 2).saturating_add(60))
                    }
                    _ => (a.genome[AFF], 255u8.saturating_sub(a.genome[METAB]), a.genome[AGGR]),
                };
                app.frame[o] = r;
                app.frame[o + 1] = g;
                app.frame[o + 2] = b;
                app.frame[o + 3] = 255;
            }
            None => {
                let res = app.world.resource[i] as u32;
                let glow = (16 + res * 150 / cap_max).min(150) as u8;
                app.frame[o] = 3;
                app.frame[o + 1] = glow / 2 + 6;
                app.frame[o + 2] = glow;
                app.frame[o + 3] = 255;
            }
        }
    }
    app.frame.as_ptr()
}

#[no_mangle]
pub extern "C" fn width(app: *mut App) -> u32 {
    unsafe { (*app).w as u32 }
}
#[no_mangle]
pub extern "C" fn height(app: *mut App) -> u32 {
    unsafe { (*app).h as u32 }
}
#[no_mangle]
pub extern "C" fn epoch(app: *mut App) -> u32 {
    unsafe { (*app).world.epoch as u32 }
}
#[no_mangle]
pub extern "C" fn population(app: *mut App) -> u32 {
    let app = unsafe { &*app };
    app.world.cells.iter().filter(|c| c.is_some()).count() as u32
}
/// Cumulative births (divisions). JS diffs across frames for a live rate.
#[no_mangle]
pub extern "C" fn births(app: *mut App) -> u32 {
    unsafe { (*app).world.births as u32 }
}
/// Cumulative deaths.
#[no_mangle]
pub extern "C" fn deaths(app: *mut App) -> u32 {
    unsafe { (*app).world.deaths as u32 }
}

/// Mean of gene `g` over living agents, in 0..=255.
#[no_mangle]
pub extern "C" fn gene_mean(app: *mut App, g: u32) -> f32 {
    let app = unsafe { &*app };
    let g = g as usize;
    let mut sum = 0u64;
    let mut pop = 0u64;
    for c in app.world.cells.iter() {
        if let Some(a) = c {
            sum += a.genome[g] as u64;
            pop += 1;
        }
    }
    if pop == 0 {
        0.0
    } else {
        sum as f32 / pop as f32
    }
}

/// Mean generation depth over living agents.
#[no_mangle]
pub extern "C" fn mean_gen(app: *mut App) -> f32 {
    let app = unsafe { &*app };
    let mut sum = 0u64;
    let mut pop = 0u64;
    for a in app.world.cells.iter().flatten() {
        sum += a.gen as u64;
        pop += 1;
    }
    if pop == 0 {
        0.0
    } else {
        sum as f32 / pop as f32
    }
}

/// Count of distinct living lineages (founding strains still represented).
#[no_mangle]
pub extern "C" fn strains(app: *mut App) -> u32 {
    let app = unsafe { &*app };
    let n = app.world.cfg.init_agents.max(1);
    let mut seen = vec![false; n];
    let mut count = 0u32;
    for a in app.world.cells.iter().flatten() {
        let s = a.strain as usize;
        if s < n && !seen[s] {
            seen[s] = true;
            count += 1;
        }
    }
    count
}

#[no_mangle]
pub extern "C" fn free_app(app: *mut App) {
    if !app.is_null() {
        unsafe { drop(Box::from_raw(app)) };
    }
}
