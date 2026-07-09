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
#[no_mangle]
pub extern "C" fn init(seed_lo: u32, seed_hi: u32, width: u32, height: u32, init_agents: u32) -> *mut App {
    let w = width as usize;
    let h = height as usize;
    let cfg = Config {
        width: w,
        height: h,
        init_agents: init_agents as usize,
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

/// Paint the grid into the RGBA buffer and return a pointer to it.
/// Live cells are coloured by genome (R=affinity, G=efficiency=255−metab,
/// B=aggression) so the population's colour visibly shifts as it evolves;
/// empty cells glow teal by resource, revealing the sugarscape peaks.
#[no_mangle]
pub extern "C" fn render(app: *mut App) -> *const u8 {
    let app = unsafe { &mut *app };
    let cap_max = app.world.cfg.cap_max.max(1) as u32;
    for i in 0..app.w * app.h {
        let o = i * 4;
        match &app.world.cells[i] {
            Some(a) => {
                app.frame[o] = a.genome[AFF];
                app.frame[o + 1] = 255u8.saturating_sub(a.genome[METAB]);
                app.frame[o + 2] = a.genome[AGGR];
                app.frame[o + 3] = 255;
            }
            None => {
                let res = app.world.resource[i] as u32;
                let glow = (18 + res * 150 / cap_max).min(150) as u8;
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
