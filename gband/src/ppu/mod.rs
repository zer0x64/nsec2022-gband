use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

mod cgb_palette;
mod fifo_mode;
mod lcd_control;
mod lcd_status;
mod pixel_fifo;

use cgb_palette::CgbPalette;
use fifo_mode::FifoMode;
use lcd_control::LcdControl;
use lcd_status::LcdStatus;

use crate::bus::PpuBus;
use crate::InterruptReg;

use self::{
    fifo_mode::{OamScanState, PixelFetcherState},
    pixel_fifo::PixelFifo,
};

pub const FRAME_WIDTH: usize = 160;
pub const FRAME_HEIGHT: usize = 144;

pub type Frame = Box<[u8; FRAME_WIDTH * FRAME_HEIGHT * 4]>;

pub struct Ppu {
    x: u8,
    y: u8,
    window_y_counter: u8,
    window_y_flag: bool,
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

    background_pixel_pipeline: PixelFifo,
    sprite_pixel_pipeline: PixelFifo,

    cycle: u16,
    fifo_mode: FifoMode,
    frame: Frame,
}

impl Default for Ppu {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            window_y_counter: 0,
            window_y_flag: false,
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

            background_pixel_pipeline: Default::default(),
            sprite_pixel_pipeline: Default::default(),

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

        if self.y < 144 {
            match self.cycle {
                80 => {
                    self.fifo_mode = FifoMode::Drawing(Default::default());
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
                144..=153 => {
                    // We are in VBLANK
                    self.fifo_mode = FifoMode::VBlank;

                    if self.y == 144 {
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
                    self.window_y_counter = 0;
                    self.window_y_flag = false;
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

        self.render(bus);
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
            FifoMode::Drawing(_) => {
                // Calls are blocked during this mode
                // Do nothing
                // TODO: There are timing issues right now so the write block breaks rendering right now.
                // Delete those lines when the timing issues are fixed
                let addr = addr & 0x1FFF | if self.vram_bank_register { 0x2000 } else { 0 };
                self.vram[addr as usize] = data;
            }
            _ => {
                let addr = addr & 0x1FFF | if self.vram_bank_register { 0x2000 } else { 0 };
                self.vram[addr as usize] = data;
            }
        }
    }

    pub fn read_vram(&self, addr: u16) -> u8 {
        match self.fifo_mode {
            FifoMode::Drawing(_) => {
                // Calls are blocked during this mode
                // Do nothing and return trash
                0xFF
            }
            _ => self.read_vram_unblocked(addr),
        }
    }

    fn read_vram_unblocked(&self, addr: u16) -> u8 {
        let addr = addr & 0x1FFF | if self.vram_bank_register { 0x2000 } else { 0 };
        self.vram[addr as usize]
    }

    pub fn write_oam(&mut self, addr: u16, data: u8, force: bool) {
        // TODO: Redo write block
        // match self.fifo_mode {
        //     FifoMode::OamScan { .. } | FifoMode::Drawing(_) => {
        //         // Calls are blocked during this mode
        //         // Do nothing, except if this is called by the OAM DMA
        //         if !force {
        //             return;
        //         }
        //     }
        //     _ => {
        //         // Continue normally
        //     }
        // }

        let addr = addr & 0xFF;
        self.oam[addr as usize] = data;
    }

    pub fn read_oam(&self, addr: u16, force: bool) -> u8 {
        // TODO: Redo read block
        // match self.fifo_mode {
        //     FifoMode::OamScan(_) | FifoMode::Drawing(_) => {
        //         // Calls are blocked during this mode
        //         // Do nothing and return trash, except if this is called by the OAM DMA
        //         if !force {
        //             return 0xFF;
        //         }
        //     }
        //     _ => {
        //         // Continue normally
        //     }
        // }

        let addr = addr & 0xFF;
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

    fn render(&mut self, bus: &mut PpuBus) {
        // Work on a copy to fix borrow checker issue
        let mut fifo_mode = self.fifo_mode;

        match &mut fifo_mode {
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

                    // The index is y + 16, so the sprite can be hidden off at 0. This is why we add 16 here
                    let y_remainder = self.y.wrapping_sub(y).wrapping_add(16);

                    *is_visible = (y_remainder < sprite_size) && (self.oam[*oam_pointer + 1] > 0);
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
            FifoMode::Drawing(state) => {
                // if self
                //     .lcd_control_reg
                //     .contains(LcdControl::BACKGROUND_WINDOW_ENABLE_PRIORITY)
                // {
                // NOTE: assuming non-GBC mode only for now

                // Check for window
                if self.y == self.window_y {
                    self.window_y_flag = true;
                }

                if !state.is_window && self.lcd_control_reg.contains(LcdControl::WINDOW_ENABLE) {
                    if self.window_y_flag && self.x >= self.window_x.wrapping_sub(7) {
                        // We start rendering the window
                        // We flush the entire state and signal that we start to render the window
                        state.reset();

                        state.is_window = true;
                        state.fetcher_x = 0;
                    }
                }

                // Check for sprites
                if !state.is_sprite && self.lcd_control_reg.contains(LcdControl::OBJ_ENABLE) {
                    // This condition is only for when on DMG!
                    for (index, sprite) in self.secondary_oam.chunks_exact(4).enumerate() {
                        let sprite = <&[u8; 4]>::try_from(sprite)
                            .expect("secondary OAM should always be chunks of 4");

                        // The sprite address is x + 8, so it can be hidden if set at 0
                        let x_remainder = self.x.wrapping_sub(sprite[1]).wrapping_add(8);
                        if x_remainder < 8 {
                            // Start a sprite fetch
                            state.reset();

                            state.is_sprite = true;
                            state.sprite_idx = (index << 2) as u8;

                            break;
                        }
                    }
                }

                match state.pixel_fetcher {
                    PixelFetcherState::GetTile => {
                        // Get the tile used in this part of the map
                        // Here we use a specific fetcher indexing
                        // https://gbdev.io/pandocs/pixel_fifo.html
                        if state.cycle == 0 {
                            state.tile_idx = if state.is_sprite {
                                // For sprites, we simply fetch it from the OAM entry
                                self.secondary_oam[(state.sprite_idx + 2) as usize]
                            } else if state.is_window {
                                // For window, we use the internal window Y counter and the X fetch counter
                                let x_index =
                                    (state.fetcher_x.wrapping_sub(self.window_x >> 3)) & 0x1F;
                                let y_index = self.window_y_counter >> 3;
                                let tile_map_idx =
                                    ((y_index as u16) << 5) | (x_index as u16 & 0x1F);

                                self.read_win_tile_index(tile_map_idx)
                            } else {
                                // For background, we use the scanline number as Y and the X fetch counter
                                let x_index = ((self.scroll_x >> 3) + (state.fetcher_x)) & 0x1F;
                                let y_index = self.y.wrapping_add(self.scroll_y) >> 3;
                                let tile_map_idx = ((y_index as u16) << 5) | (x_index as u16);

                                self.read_bg_tile_index(tile_map_idx)
                            };

                            state.cycle += 1;
                        } else {
                            state.pixel_fetcher = PixelFetcherState::GetTileLow;
                            state.cycle = 0;
                        }
                    }
                    PixelFetcherState::GetTileLow => {
                        // Get the low bits of the used palette
                        if state.cycle == 0 {
                            let mut tile_data = if state.is_sprite {
                                let sprite_size =
                                    if self.lcd_control_reg.contains(LcdControl::OBJ_SIZE) {
                                        15
                                    } else {
                                        7
                                    };

                                let mut fine_y = self
                                    .y
                                    .wrapping_sub(self.secondary_oam[state.sprite_idx as usize])
                                    .wrapping_add(16)
                                    & sprite_size;

                                // Y flip
                                if self.secondary_oam[(state.sprite_idx + 3) as usize] & 0x40 > 0 {
                                    fine_y = sprite_size - fine_y;
                                }

                                // For 8x16 sprites, get the right index
                                let tile_id = if self.lcd_control_reg.contains(LcdControl::OBJ_SIZE)
                                {
                                    (state.tile_idx & 0xFE) | ((fine_y & 0x08) >> 3)
                                } else {
                                    state.tile_idx
                                };

                                self.read_obj_tile(tile_id, fine_y << 1)
                            } else {
                                let row = if state.is_window {
                                    // For sprite, we select using the internal window Y counter
                                    self.window_y_counter & 0x7
                                } else {
                                    // For background, we select using the scanline number as Y
                                    self.y.wrapping_add(self.scroll_y) & 0x7
                                };

                                self.read_bg_win_tile(state.tile_idx, row << 1)
                            };

                            // Put the tile data where it belongs in the buffer
                            state.buffer = [0u16; 8];
                            for val in &mut state.buffer {
                                *val |= tile_data as u16 & 1;
                                tile_data >>= 1;
                            }

                            state.cycle += 1;
                        } else {
                            state.pixel_fetcher = PixelFetcherState::GetTileHigh;
                            state.cycle = 0;
                        }
                    }
                    PixelFetcherState::GetTileHigh => {
                        // Get the low bits of the used palette
                        if state.cycle == 0 {
                            let mut tile_data = if state.is_sprite {
                                let sprite_size =
                                    if self.lcd_control_reg.contains(LcdControl::OBJ_SIZE) {
                                        15
                                    } else {
                                        7
                                    };

                                let mut fine_y = self
                                    .y
                                    .wrapping_sub(self.secondary_oam[state.sprite_idx as usize])
                                    .wrapping_add(16)
                                    & sprite_size;

                                // Y flip
                                if self.secondary_oam[(state.sprite_idx + 3) as usize] & 0x40 > 0 {
                                    fine_y = sprite_size - fine_y;
                                }

                                // For 8x16 sprites, get the right index
                                let tile_id = if self.lcd_control_reg.contains(LcdControl::OBJ_SIZE)
                                {
                                    (state.tile_idx & 0xFE) | ((fine_y & 0x08) >> 3)
                                } else {
                                    state.tile_idx
                                };

                                self.read_obj_tile(tile_id, (fine_y << 1) | 1)
                            } else {
                                let row = if state.is_window {
                                    // For sprite, we select using the internal window Y counter
                                    self.window_y_counter & 0x7
                                } else {
                                    // For background, we select using the scanline number as Y
                                    self.y.wrapping_add(self.scroll_y) & 0x7
                                };

                                self.read_bg_win_tile(state.tile_idx, (row << 1) | 1)
                            };

                            // Put the tile data where it belongs in the buffer
                            for val in &mut state.buffer {
                                *val |= (tile_data as u16 & 1) << 1;
                                tile_data >>= 1;
                            }

                            state.cycle += 1;
                        } else {
                            state.pixel_fetcher = PixelFetcherState::Push;
                            state.cycle = 0;
                        }
                    }
                    PixelFetcherState::Push => {
                        if state.is_sprite {
                            let sprite_properties = self.secondary_oam[(state.sprite_idx + 3) as usize];
                            // X flip
                            if sprite_properties & 0x20 > 0 {
                                state.buffer.reverse();
                            }

                            // Add palette and priority bits
                            for b in &mut state.buffer {
                                *b |= (sprite_properties & 0x90) as u16;
                            }

                            self.sprite_pixel_pipeline.load(state.buffer);

                            if self.x == 0 {
                                for _ in
                                    0..(8 - self.secondary_oam[(state.sprite_idx + 1) as usize])
                                {
                                    // Drain the extra pixels
                                    let _ = self.sprite_pixel_pipeline.pop();
                                }
                            }

                            state.pixel_fetcher = PixelFetcherState::GetTile;
                            state.is_sprite = false;

                            // Remove the sprite
                            self.secondary_oam[(state.sprite_idx + 1) as usize] = 0;
                        } else {
                            if self.background_pixel_pipeline.is_empty() {
                                // Hang until pipeline is empty to load it
                                self.background_pixel_pipeline.load(state.buffer);

                                if !state.is_window {
                                    for _ in 0..(self.scroll_x.wrapping_add(self.x)) & 0x7 {
                                        // Drain the extra pixels
                                        let _ = self.background_pixel_pipeline.pop();
                                    }
                                }

                                state.pixel_fetcher = PixelFetcherState::GetTile;
                                state.fetcher_x += 1;
                            }
                        }
                    }
                }

                // Rendering...
                if !self.background_pixel_pipeline.is_empty() & !state.is_sprite {
                    let background_pixel = self.background_pixel_pipeline.pop();
                    let sprite_pixel = self.sprite_pixel_pipeline.pop();

                    let sprite_palette = (sprite_pixel as usize & 0x10) >> 4;
                    let bg_over_obj = (sprite_pixel & 0x80) == 0x80;

                    let pixel = if !self.lcd_control_reg.contains(LcdControl::OBJ_ENABLE)
                        || (sprite_pixel & 3 == 0)
                        || (bg_over_obj && (background_pixel & 3 != 0))
                    {
                        // Pixel is transparent, under the background or LCDC.1 is disabled. Rendering background instead
                        // Index the pixel in the palette
                        if self.lcd_control_reg.contains(LcdControl::BACKGROUND_WINDOW_ENABLE_PRIORITY) {
                            (self.greyscale_bg_palette >> ((background_pixel as u8 & 0x3) << 1)) & 0x3
                        } else {
                            // Very simple and potentially incomplete implementation of LCDC.0 for DMG. For CGB, there should be more to do as well.
                            self.greyscale_bg_palette & 0x3
                        }                    
                    } else {
                        // Renderng the sprite pixel
                        // Index the pixel in the palette
                        (self.greyscale_obj_palette[sprite_palette]
                            >> ((sprite_pixel as u8 & 0x3) << 1))
                            & 0x3
                    };

                    let base = ((self.y as usize) * FRAME_WIDTH + (self.x as usize)) * 4;
                    if base + 3 < self.frame.len() {
                        // Convert to RGBA
                        let greyscale = !(pixel as u8 & 3) << 6;
                        self.frame[base] = greyscale;
                        self.frame[base + 1] = greyscale;
                        self.frame[base + 2] = greyscale;
                        self.frame[base + 3] = greyscale;

                        self.x += 1;

                        if self.x >= FRAME_WIDTH as u8 {
                            // We enter HBlank here

                            // Reset some buffers
                            self.background_pixel_pipeline = Default::default();
                            self.sprite_pixel_pipeline = Default::default();
                            self.secondary_oam = [0u8; 40];

                            if state.is_window {
                                self.window_y_counter += 1;
                            };

                            fifo_mode = FifoMode::HBlank;

                            if self
                                .lcd_status_reg
                                .contains(LcdStatus::HBANLK_INTERUPT_SOURCE)
                            {
                                bus.request_interrupt(InterruptReg::LCD_STAT);
                            }
                        };
                    }
                }
            }
            _ => {
                // Don't render anything in HBLANK/VBLANK
            }
        }

        self.fifo_mode = fifo_mode;
    }

    fn read_bg_win_tile(&self, id: u8, offset: u8) -> u8 {
        // See: https://gbdev.io/pandocs/Tile_Data.html
        if self
            .lcd_control_reg
            .contains(LcdControl::BACKGROUND_WINDOW_TILE_DATA_AREA)
        {
            self.read_obj_tile(id, offset)
        } else {
            let is_id_negative = id & 0x80 == 0x80;

            let addr_to_read = if !is_id_negative {
                0x9000 | ((id as u16) << 4) | (offset as u16)
            } else {
                0x8800 | (((id as u16) << 4) & 0x7FF) | (offset as u16)
            };
            self.read_vram_unblocked(addr_to_read)
        }
    }

    fn read_obj_tile(&self, id: u8, offset: u8) -> u8 {
        let base_addr = 0x8000;
        let addr_to_read = base_addr | (u16::from(id) << 4) | offset as u16;
        self.read_vram_unblocked(addr_to_read)
    }

    fn read_bg_tile_index(&self, id: u16) -> u8 {
        // See: https://gbdev.io/pandocs/Tile_Maps.html
        if self
            .lcd_control_reg
            .contains(LcdControl::BACKGROUND_TILE_MAP_AREA)
        {
            let addr = 0x9C00 | id;
            self.read_vram_unblocked(addr)
        } else {
            let addr = 0x9800 | id;
            self.read_vram_unblocked(addr)
        }
    }

    fn read_win_tile_index(&self, id: u16) -> u8 {
        // See: https://gbdev.io/pandocs/Tile_Maps.html
        if self
            .lcd_control_reg
            .contains(LcdControl::WINDOW_TILE_MAP_AREA)
        {
            let addr = 0x9C00 | id;
            self.read_vram_unblocked(addr)
        } else {
            let addr = 0x9800 | id;
            self.read_vram_unblocked(addr)
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
