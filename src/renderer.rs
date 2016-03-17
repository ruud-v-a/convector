use ray::{MIntersection, MRay};
use scene::{Light, Scene};
use simd::{Mf32, Mi32};
use std::cell::UnsafeCell;
use std::mem;
use time::PreciseTime;
use util;
use vector3::{MVector3, SVector3};

pub struct Renderer {
    scene: Scene,
    width: u32,
    height: u32,
    epoch: PreciseTime,
}

/// The buffer that an image is rendered into.
pub struct RenderBuffer {
    buffer: UnsafeCell<Vec<Mi32>>,
}

impl RenderBuffer {
    /// Allocates a new buffer to render into, memory uninitialized.
    ///
    /// The width and height must be a multiple of 16.
    pub fn new(width: u32, height: u32) -> RenderBuffer {
        assert_eq!(width & 15, 0);  // Width must be a multiple of 16.
        assert_eq!(height & 15, 0); // Height must be a multiple of 16.

        // There are 8 RGBA pixels in one mi32.
        let num_elems = (width as usize) * (height as usize) / 8;

        let mut vec = util::cache_line_aligned_vec(num_elems);
        unsafe { vec.set_len(num_elems); }

        RenderBuffer {
            buffer: UnsafeCell::new(vec),
        }
    }

    /// Zeroes the buffer.
    pub fn fill_black(&mut self) {
        // This is actually safe because self is borrowed mutably.
        for pixels in unsafe { self.get_mut_slice() } {
            *pixels = Mi32::zero();
        }
    }

    /// Returns a mutable view into the buffer.
    ///
    /// This is unsafe because it allows creating multiple mutable borrows of
    /// the buffer, which could result in races. Threads should ensure that
    /// they write to disjoint parts of the buffer.
    pub unsafe fn get_mut_slice(&self) -> &mut [Mi32] {
        (*self.buffer.get()).as_mut_slice()
    }

    /// Returns an RGBA bitmap suitable for display.
    pub fn into_bitmap(self) -> Vec<u8> {
        // This is actually safe because self is moved into the method.
        let mut buffer = unsafe { self.buffer.into_inner() };
        let mi32_ptr = buffer.as_mut_ptr();
        let num_bytes = buffer.len() * 32; // Mi32 is 8 pixels of 4 bytes.

        // Prevent the destructor of the buffer from freeing the memory.
        mem::forget(buffer);

        // Transmute the vector into a vector of bytes.
        unsafe {
            let u8_ptr = mem::transmute(mi32_ptr);
            Vec::from_raw_parts(u8_ptr, num_bytes, num_bytes)
        }
    }
}

// The render buffer must be shared among threads, but UnsafeCell is not Sync.
unsafe impl Sync for RenderBuffer { }

/// Builds a fixed-size slice by calling f for every index.
fn generate_slice8<T, F>(mut f: F) -> [T; 8] where F: FnMut(usize) -> T {
    [f(0), f(1), f(2), f(3), f(4), f(5), f(6), f(7)]
}

impl Renderer {
    pub fn new(scene: Scene, width: u32, height: u32) -> Renderer {
        Renderer {
            scene: scene,
            width: width,
            height: height,
            epoch: PreciseTime::now(),
        }
    }

    /// For an interactive scene, updates the scene for the new frame.
    /// TODO: This method does not really belong here.
    pub fn update_scene(&mut self) {
        let t = self.epoch.to(PreciseTime::now()).num_milliseconds() as f32 * 1e-3;

        // Make the light circle around.
        self.scene.lights[0].position = SVector3 {
            x: t.cos() * 5.0,
            y: (t * 0.3).cos() * 7.0,
            z: t.sin() * 5.0,
        };
    }

    /// Returns the screen coordinates of the block of 16x4 pixels where (x, y)
    /// is the bottom-left coordinate. The order is as follows:
    ///
    ///     0c 0d 0e 0f  1c 1d 1e 1f  2c 2d 2e 2f  3c 3d 3e 3f
    ///     08 09 0a 0b  18 19 1a 1b  28 29 2a 2b  38 39 3a 3b
    ///     04 05 06 07  14 15 16 17  24 25 26 27  34 35 36 37
    ///     00 01 02 03  10 11 12 13  20 21 22 23  30 31 32 33
    ///
    /// Or, in terms of the mf32s:
    ///
    ///     1 1 1 1  3 3 3 3  5 5 5 5  7 7 7 7
    ///     1 1 1 1  3 3 3 3  5 5 5 5  7 7 7 7
    ///     0 0 0 0  2 2 2 2  4 4 4 4  6 6 6 6
    ///     0 0 0 0  2 2 2 2  4 4 4 4  6 6 6 6
    ///
    /// Where inside every mf32 the pixels are ordered from left to right,
    /// bottom to top.
    fn get_pixel_coords_16x4(&self, x: u32, y: u32) -> ([Mf32; 8], [Mf32; 8]) {
        let scale = Mf32::broadcast(2.0 / self.width as f32);
        let scale_mul = Mf32(2.0, 4.0, 8.0, 12.0, 0.0, 0.0, 0.0, 0.0) * scale;

        let off_x = Mf32(0.0, 1.0, 2.0, 3.0, 0.0, 1.0, 2.0, 3.0);
        let off_y = Mf32(0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0);

        let base_x = scale * (off_x + Mf32::broadcast(x as f32 - self.width as f32 * 0.5));
        let base_y = scale * (off_y + Mf32::broadcast(y as f32 - self.height as f32 * 0.5));

        let xs = [
            base_x,
            base_x,
            base_x + Mf32::broadcast(scale_mul.1), // 4.0 * scale
            base_x + Mf32::broadcast(scale_mul.1), // 4.0 * scale
            base_x + Mf32::broadcast(scale_mul.2), // 8.0 * scale
            base_x + Mf32::broadcast(scale_mul.2), // 8.0 * scale
            base_x + Mf32::broadcast(scale_mul.3), // 12.0 * scale
            base_x + Mf32::broadcast(scale_mul.3)  // 12.0 * scale
        ];

        let ys = [
            base_y, base_y + Mf32::broadcast(scale_mul.0), // 2.0 * scale
            base_y, base_y + Mf32::broadcast(scale_mul.0), // 2.0 * scale
            base_y, base_y + Mf32::broadcast(scale_mul.0), // 2.0 * scale
            base_y, base_y + Mf32::broadcast(scale_mul.0)  // 2.0 * scale
        ];

        (xs, ys)
    }

    /// Renders a block of 16x4 pixels, where (x, y) is the coordinate of the
    /// bottom-left pixel. Bitmap must be an array of 8 pixels at once, and it
    /// must be aligned to 64 bytes (a cache line).
    fn render_block_16x4(&self, bitmap: &mut [Mi32], x: u32, y: u32) {
        // Render pixels, get f32 colors.
        let (xs, ys) = self.get_pixel_coords_16x4(x, y);
        let rgbs = generate_slice8(|i| self.render_pixels(xs[i], ys[i]));

        // Convert f32 colors to i32 colors in the range 0-255.
        let range = Mf32::broadcast(255.0);
        let rgbas = generate_slice8(|i| {
            let rgb_255 = rgbs[i].clamp_one() * range;
            let r = rgb_255.x.into_mi32();
            let g = rgb_255.y.into_mi32().map(|x| x << 8);
            let b = rgb_255.z.into_mi32().map(|x| x << 16);
            let a = Mi32::broadcast(0xff000000_u32 as i32);
            (r | g) | (b | a)
        });

        // Helper functions to shuffle around the pixels from the order as
        // described in `get_pixel_coords_16x4` into four rows of 16 pixels.
        let mk_line0 = |left: Mi32, right: Mi32|
            Mi32(left.0, left.1, left.2, left.3, right.0, right.1, right.2, right.3);
        let mk_line1 = |left: Mi32, right: Mi32|
            Mi32(left.4, left.5, left.6, left.7, right.4, right.5, right.6, right.7);

        // Store the pixels in the bitmap. If the bitmap is aligned to the cache
        // line size, this stores exactly four cache lines, so there is no need
        // to fetch those lines because all bytes are overwritten. This saves a
        // trip to memory, which makes this store fast.
        let idx_line0 = ((y * self.width + 0 * self.width + x) / 8) as usize;
        let idx_line1 = ((y * self.width + 1 * self.width + x) / 8) as usize;
        let idx_line2 = ((y * self.width + 2 * self.width + x) / 8) as usize;
        let idx_line3 = ((y * self.width + 3 * self.width + x) / 8) as usize;
        bitmap[idx_line0 + 0] = mk_line0(rgbas[0], rgbas[2]);
        bitmap[idx_line0 + 1] = mk_line0(rgbas[4], rgbas[6]);
        bitmap[idx_line1 + 0] = mk_line1(rgbas[0], rgbas[2]);
        bitmap[idx_line1 + 1] = mk_line1(rgbas[4], rgbas[6]);
        bitmap[idx_line2 + 0] = mk_line0(rgbas[1], rgbas[3]);
        bitmap[idx_line2 + 1] = mk_line0(rgbas[5], rgbas[7]);
        bitmap[idx_line3 + 0] = mk_line1(rgbas[1], rgbas[3]);
        bitmap[idx_line3 + 1] = mk_line1(rgbas[5], rgbas[7]);
    }

    /// Renders a square part of a frame.
    ///
    /// The (x, y) coordinate is the coordinate of the bottom-left pixel of the
    /// patch. The patch width must be a multiple of 16.
    pub fn render_patch(&self, bitmap: &mut [Mi32], patch_width: u32, x: u32, y: u32) {
        assert_eq!(patch_width & 15, 0); // Patch width must be a multiple of 16.
        let h = patch_width / 4;
        let w = patch_width / 16;

        for i in 0..w {
            for j in 0..h {
                self.render_block_16x4(bitmap, x + i * 16, y + j * 4);
            }
        }
    }

    /// Returns the contribution of the light to the irradiance at the surface
    /// of intersection.
    fn get_irradiance(&self, isect: &MIntersection, light: &Light) -> Mf32 {
        // Set up a shadow ray.
        let light_pos = MVector3::broadcast(light.position);
        let to_isect = isect.position - light_pos;
        let distance_squared = to_isect.norm_squared();
        let distance = distance_squared.sqrt();

        // The inverse distance can be computed as `distance.recip()` or as
        // `distance_squared.rsqrt()`. According to the Intel intrinsics guide,
        // both an rcp and rsqrt have a latency of 7 and a throughput of 1, but
        // the rsqrt way has one less data dependency.
        let inv_dist = distance_squared.rsqrt();
        let direction = to_isect * inv_dist;
        let ray = MRay {
            origin: light_pos,
            direction: direction,
        };

        // Test for occlusion. Remove an epsilon from the max distance, to make
        // sure we don't intersect the surface we intend to shade.
        let mask = self.scene.intersect_any(&ray, distance - Mf32::epsilon());

        // Cosine of angle between surface normal and light direction, or 0 if
        // the light is behind the surface. The sign of the dot product is
        // reversed because direction goes from the light to the surface, not
        // from surface to the light.
        let cos_alpha = (Mf32::zero() - isect.normal.dot(direction)).max(Mf32::zero());

        // Power falls off as one over distance squared.
        let falloff = inv_dist * inv_dist;

        // The bitwise and could be computed with falloff, with cos_alpha, or
        // with their product. Falloff requires the least computation, so by
        // doing the bitwise and with falloff we get the shortest dependency
        // chain.
        cos_alpha * (falloff & mask)
    }

    fn render_pixels(&self, x: Mf32, y: Mf32) -> MVector3 {
        let ray = self.scene.camera.get_ray(x, y);
        let isect = self.scene.intersect_nearest(&ray);

        let mut color = MVector3::zero();

        for ref light in &self.scene.lights {
            // TODO: Do not hard-code color.
            let light_color = MVector3::new(Mf32::broadcast(20.0), Mf32::zero(), Mf32::zero());
            let irradiance = self.get_irradiance(&isect, light);
            color = light_color.mul_add(irradiance, color);
        }

        color
    }
}
