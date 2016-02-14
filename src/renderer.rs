pub struct Renderer {
    width: u32,
    height: u32,
}

impl Renderer {
    pub fn new(width: u32, height: u32) -> Renderer {
        Renderer {
            width: width,
            height: height,
        }
    }

    pub fn render(&self, dest_bitmap: &mut [u8]) {
        assert_eq!(dest_bitmap.len(), self.width as usize * self.height as usize * 4);
        for j in 0..self.height {
            for i in 0..self.width {
                let idx = ((j * self.width + i) * 4) as usize;
                dest_bitmap[idx + 0] = 255; //(255.0 * i as f32 / self.width as f32) as u8;
                dest_bitmap[idx + 1] = (256 * j / self.height) as u8;
                dest_bitmap[idx + 2] = 0;
                dest_bitmap[idx + 3] = 255;
            }
        }
    }
}