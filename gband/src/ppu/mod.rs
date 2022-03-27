use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

mod cgb_palette;
mod fifo_mode;
mod lcd_control;
mod lcd_status;

use cgb_palette::CgbPalette;
use fifo_mode::FifoMode;
use lcd_control::LcdControl;
use lcd_status::LcdStatus;

use crate::bus::PpuBus;
use crate::InterruptReg;

use self::fifo_mode::OamScanState;

pub const FRAME_WIDTH: usize = 160;
pub const FRAME_HEIGHT: usize = 144;

pub type Frame = Box<[u8; FRAME_WIDTH * FRAME_HEIGHT * 4]>;

pub struct Ppu {
    x: u16,
    y: u8,
    y_compare: u8,

    window_x: u8,
    window_y: u8,

    scroll_x: u8,
    scroll_y: u8,

    vram: [u8; 0x4000],
    vram_bank_register: bool,
    oam: [u8; 0xa0],
    secondary_oam: [u8; 40],

    cgb_bg_palette: CgbPalette,
    cgb_obj_palette: CgbPalette,

    greyscale_bg_palette: u8,
    greyscale_obj_palette: [u8; 2],

    lcd_control_reg: LcdControl,
    lcd_status_reg: LcdStatus,

    background_pixel_pipeline: u128,
    sprite_pixel_pipeline: u128,

    cycle: u16,
    fifo_mode: FifoMode,
    frame: Frame,
}

impl Default for Ppu {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            y_compare: 0,

            window_x: 0,
            window_y: 0,

            scroll_x: 0,
            scroll_y: 0,

            vram: [0u8; 0x4000],
            vram_bank_register: false,
            oam: [0u8; 0xa0],
            secondary_oam: [0u8; 40],

            lcd_control_reg: Default::default(),
            lcd_status_reg: Default::default(),

            // Boot ROM initializes the Background palettes to white
            cgb_bg_palette: CgbPalette {
                data: [0xFFu8; 0x40],
                ..Default::default()
            },
            cgb_obj_palette: CgbPalette {
                data: [0xFFu8; 0x40],
                ..Default::default()
            },

            greyscale_bg_palette: 0,
            greyscale_obj_palette: [0; 2],

            background_pixel_pipeline: 0,
            sprite_pixel_pipeline: 0,

            cycle: 0,
            fifo_mode: Default::default(),
            frame: allocate_new_frame(),
        }
    }
}

impl Ppu {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clock(&mut self, bus: &mut PpuBus) {
        self.cycle += 1;

        if self.y < 153 {
            match self.cycle {
                80 => {
                    self.fifo_mode = FifoMode::Drawing;
                }
                352 => {
                    // Hardcodes HBLANK for now
                    self.fifo_mode = FifoMode::HBlank;
                    if self
                        .lcd_status_reg
                        .contains(LcdStatus::HBANLK_INTERUPT_SOURCE)
                    {
                        bus.request_interrupt(InterruptReg::LCD_STAT);
                    }
                }
                _ => {}
            }
        }

        if self.cycle == 456 {
            self.cycle = 0;
            self.x = 0;
            self.y += 1;

            // TODO: Selection priority
            // During each scanline’s OAM scan, the PPU compares LY (using LCDC bit 2 to determine their size) to each object’s Y position to select up to 10 objects to be drawn on that line. The PPU scans OAM sequentially (from $FE00 to $FE9F), selecting the first (up to) 10 suitably-positioned objects.
            // Since the PPU only checks the Y coordinate to select objects, even off-screen objects count towards the 10-objects-per-scanline limit. Merely setting an object’s X coordinate to X = 0 or X ≥ 168 (160 + 8) will hide it, but it will still count towards the limit, possibly causing another object later in OAM not to be drawn. To keep off-screen objects from affecting on-screen ones, make sure to set their Y coordinate to Y = 0 or Y ≥ 160 (144 + 16). (Y ≤ 8 also works if object size is set to 8x8.)

            match self.y {
                143..=153 => {
                    // We are in VBLANK
                    self.fifo_mode = FifoMode::VBlank;

                    if self.y == 143 {
                        // Request VBLANK interrupt
                        bus.request_interrupt(InterruptReg::VBLANK);

                        if self
                            .lcd_status_reg
                            .contains(LcdStatus::VBANLK_INTERUPT_SOURCE)
                        {
                            bus.request_interrupt(InterruptReg::LCD_STAT);
                        }
                    }
                }
                154 => {
                    // End of the frame
                    self.y = 0;
                    self.fifo_mode = FifoMode::OamScan(Default::default());

                    if self.lcd_status_reg.contains(LcdStatus::OAM_INTERUPT_SOURCE) {
                        bus.request_interrupt(InterruptReg::LCD_STAT);
                    }
                }
                _ => {
                    self.fifo_mode = FifoMode::OamScan(Default::default());

                    if self.lcd_status_reg.contains(LcdStatus::OAM_INTERUPT_SOURCE) {
                        bus.request_interrupt(InterruptReg::LCD_STAT);
                    }
                }
            };

            if self.y == self.y_compare {
                if self
                    .lcd_status_reg
                    .contains(LcdStatus::LYC_EQ_LC_INTERUPT_SOURCE)
                {
                    bus.request_interrupt(InterruptReg::LCD_STAT);
                }
            };
        };

        self.render();
    }

    pub fn ready_frame(&mut self) -> Option<Frame> {
        if self.y == 0 && self.cycle == 0 {
            let new_frame = allocate_new_frame();

            // Replace current frame with the newly allocated one
            let frame = core::mem::replace(&mut self.frame, new_frame);

            Some(frame)
        } else {
            None
        }
    }

    pub fn write_vram(&mut self, addr: u16, data: u8) {
        match self.fifo_mode {
            FifoMode::Drawing => {
                // Calls are blocked during this mode
                // Do nothing
            }
            _ => {
                let addr = addr & 0x1FFF | if self.vram_bank_register { 0x2000 } else { 0 };
                self.vram[addr as usize] = data;
            }
        }
    }

    pub fn read_vram(&self, addr: u16) -> u8 {
        match self.fifo_mode {
            FifoMode::Drawing => {
                // Calls are blocked during this mode
                // Do nothing and return trash
                0xFF
            }
            _ => {
                let addr = addr & 0x1FFF | if self.vram_bank_register { 0x2000 } else { 0 };
                self.vram[addr as usize]
            }
        }
    }

    pub fn write_oam(&mut self, addr: u16, data: u8, force: bool) {
        match self.fifo_mode {
            FifoMode::OamScan { .. } | FifoMode::Drawing => {
                // Calls are blocked during this mode
                // Do nothing, except if this is called by the OAM DMA
                if !force {
                    return;
                }
            }
            _ => {
                // Continue normally
            }
        }

        let addr = addr & 0x7F;
        self.oam[addr as usize] = data;
    }

    pub fn read_oam(&self, addr: u16, force: bool) -> u8 {
        match self.fifo_mode {
            FifoMode::OamScan { .. } | FifoMode::Drawing => {
                // Calls are blocked during this mode
                // Do nothing and return trash, except if this is called by the OAM DMA
                if !force {
                    return 0xFF;
                }
            }
            _ => {
                // Continue normally
            }
        }

        let addr = addr & 0x7F;
        self.oam[addr as usize]
    }

    pub fn write(&mut self, addr: u16, data: u8) {
        match addr {
            0xFF40 => self.write_lcd_control(data),
            0xFF41 => self.write_lcd_status(data),
            0xFF42 => self.scroll_y = data,
            0xFF43 => self.scroll_x = data,
            0xFF44 => {
                // ly is Read-Only
            }
            0xFF45 => self.y_compare = data,
            0xFF47 => self.greyscale_bg_palette = data,
            0xFF48 | 0xFF49 => self.greyscale_obj_palette[(addr & 1) as usize] = data,
            0xFF4A => self.window_y = data,
            0xFF4B => self.window_x = data,
            0xFF4C => {
                // rKEY0 is blocked after boot
            }
            0xFF68 => self.cgb_bg_palette.write_spec(data),
            0xFF69 => self.cgb_bg_palette.write_data(data, self.fifo_mode),
            0xFF6A => self.cgb_obj_palette.write_spec(data),
            0xFF6B => self.cgb_obj_palette.write_data(data, self.fifo_mode),
            _ => {
                // Address not recognised, do nothing
            }
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.read_lcd_control(),
            0xFF41 => self.read_lcd_status(),
            0xFF42 => self.scroll_y,
            0xFF43 => self.scroll_x,
            0xFF44 => self.y,
            0xFF45 => self.y_compare,
            0xFF47 => self.greyscale_bg_palette,
            0xFF48 | 0xFF49 => self.greyscale_obj_palette[(addr & 1) as usize],
            0xFF4A => self.window_y,
            0xFF4B => self.window_x,
            0xFF4C => {
                // rKEY0 is blocked after boot
                0xFF
            }
            0xFF68 => self.cgb_bg_palette.read_spec(),
            0xFF69 => self.cgb_bg_palette.read_data(self.fifo_mode),
            0xFF6A => self.cgb_obj_palette.read_spec(),
            0xFF6B => self.cgb_obj_palette.read_data(self.fifo_mode),
            _ => {
                // Address not recognised, do nothing
                0
            }
        }
    }

    fn write_lcd_control(&mut self, data: u8) {
        self.lcd_control_reg =
            LcdControl::from_bits(data).expect("any data should be valid for LCDC bitflags")
    }

    fn read_lcd_control(&self) -> u8 {
        self.lcd_control_reg.bits()
    }

    fn write_lcd_status(&mut self, data: u8) {
        // Only those bits are writeable.
        let mask = 0b01111000;
        let status_reg = self.lcd_status_reg.bits() & !mask;
        let status_reg = status_reg | (data & mask);

        self.lcd_status_reg = LcdStatus::from_bits(status_reg)
            .expect("the reg can take 8 bits, so no value should fail");
    }

    fn read_lcd_status(&self) -> u8 {
        let mut status_reg = self.lcd_status_reg;

        // Those bits are constantly changed, so might as well update them only when needed
        status_reg.set(LcdStatus::LYC_EQ_LC, self.y == self.y_compare);
        status_reg.set_mode(self.fifo_mode);

        status_reg.bits()
    }

    fn render(&mut self) {
        match &mut self.fifo_mode {
            FifoMode::OamScan(OamScanState {
                oam_pointer,
                secondary_oam_pointer,
                is_visible,
            }) => {
                if self.cycle & 1 == 0 {
                    // On even cycle, fetch the y value and check if it's visible
                    let y = self.oam[*oam_pointer];

                    let sprite_size = if self.lcd_control_reg.contains(LcdControl::OBJ_SIZE) {
                        16
                    } else {
                        8
                    };
                    let fine_y = self.y.wrapping_sub(y);

                    *is_visible = fine_y < sprite_size;
                } else {
                    // On odd cycle, copy it to the secondary OAM

                    if *is_visible {
                        // Line is visible
                        if *secondary_oam_pointer < self.secondary_oam.len() {
                            self.secondary_oam[*secondary_oam_pointer..*secondary_oam_pointer + 4]
                                .copy_from_slice(&self.oam[*oam_pointer..*oam_pointer + 4]);
                            *secondary_oam_pointer += 4;
                        }
                    }

                    *oam_pointer += 4
                }
            }
            FifoMode::Drawing => {}
            _ => {
                // Don't render anything in HBLANK/VBLANK
            }
        }

        if self
            .lcd_control_reg
            .contains(LcdControl::BACKGROUND_WINDOW_ENABLE_PRIORITY)
        {
            // NOTE: assuming non-GBC mode only for now

            // Scroll using SCX and SCY registers
            let bg_x = (self.x + u16::from(self.scroll_x)) as u8; // FIXME: check if `self.x` should not rather be a `u8`
            let bg_y = self.y + self.scroll_y;

            // TODO: write background

            if self.lcd_control_reg.contains(LcdControl::WINDOW_ENABLE) {
                // Position window using WX and WY registers
                let win_x = (self.x + u16::from(self.window_x)) as u8; // FIXME: check if `self.x` should not rather be a `u8`
                let win_y = self.y + self.window_y;

                // "Window visibility"
                // https://gbdev.io/pandocs/Scrolling.html#ff4a---wy-window-y-position-rw-ff4b---wx-window-x-position--7-rw

                // Window internal line counter
                // > The window keeps an internal line counter that’s functionally similar to LY, and increments alongside it. However, it only gets incremented when the window is visible, as described here.
                // https://gbdev.io/pandocs/Tile_Maps.html#window
                // https://gbdev.io/pandocs/Scrolling.html#ff4a---wy-window-y-position-rw-ff4b---wx-window-x-position--7-rw

                // TODO: write window
            }
        }
    }

    // FIXME: temporary naming for these functions (there are 16 bytes to read to actual find pixel
    // color… refer to wiki anyway)

    fn read_sprite_attribute(&self, id: u8) {
        const SPRITE_ATTRIBUTE_TABLE_BASE_ADDR: u16 = 0xFE00;
        const SPRITE_ATTRIBUTE_SIZE: u8 = 4;

        let addr_to_read =
            SPRITE_ATTRIBUTE_TABLE_BASE_ADDR + u16::from(id) * u16::from(SPRITE_ATTRIBUTE_SIZE);

        let y_position = self.read(addr_to_read) + 16;
        let x_position = self.read(addr_to_read + 1) + 8;
        let tile_index = self.read(addr_to_read + 2) + 8;
        let flags = self.read(addr_to_read + 3) + 8;
    }

    fn read_bg_win_tile(&self, id: u8) -> u8 {
        // See: https://gbdev.io/pandocs/Tile_Data.html
        if self
            .lcd_control_reg
            .contains(LcdControl::BACKGROUND_WINDOW_TILE_DATA_AREA)
        {
            self.read_obj_tile(id)
        } else {
            let id = id as i8;
            let base_addr = 0x9000;
            let addr_to_read = if let Ok(offset) = u16::try_from(id) {
                base_addr + offset * 16
            } else {
                base_addr - u16::try_from(-id).unwrap() * 16
            };
            self.read_vram(addr_to_read)
        }
    }

    fn read_obj_tile(&self, id: u8) -> u8 {
        let base_addr = 0x8000;
        let addr_to_read = base_addr + u16::from(id) * 16;
        self.read_vram(addr_to_read)
    }

    fn read_tile_index(&self, id: u16) -> u8 {
        // See: https://gbdev.io/pandocs/Tile_Maps.html
        if self
            .lcd_control_reg
            .contains(LcdControl::BACKGROUND_TILE_MAP_AREA)
        {
            let addr = 0x9C00 + id;
            self.read_vram(addr)
        } else {
            let addr = 0x9800 + id;
            self.read_vram(addr)
        }
    }
}

fn allocate_new_frame() -> Frame {
    //   Hackish way to create fixed size boxed array.
    // I don't know of any way to do it without
    // having the data allocated on the stack at some point or using unsafe
    unsafe {
        // Safety: allocated vector has the right size for a frame array
        // (that is `FRAME_WIDTH * FRAME_HEIGHT`)
        let v: Vec<u8> = vec![0u8; FRAME_WIDTH * FRAME_HEIGHT * 4];
        Box::from_raw(
            Box::into_raw(v.into_boxed_slice()) as *mut [u8; FRAME_WIDTH * FRAME_HEIGHT * 4]
        )
    }
}
