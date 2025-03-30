use std::sync::{Arc, Mutex};

use ble::RadarBle;
use embedded_graphics::{
    draw_target::DrawTarget,
    geometry::Point,
    pixelcolor::Rgb565,
    primitives::{Circle, Primitive, PrimitiveStyle},
    Drawable,
};
use esp32_nimble::BLEDevice;
use esp_idf_hal::{
    prelude::*,
    sys::{esp_lcd_panel_disp_on_off, esp_lcd_panel_draw_bitmap},
    task::thread::ThreadSpawnConfiguration,
};
use esp_idf_svc::{
    hal::{
        ledc::{config, LedcDriver, LedcTimerDriver},
        peripherals::Peripherals,
    },
    sys::{
        esp_lcd_panel_handle_t, esp_lcd_panel_t, esp_lcd_rgb_panel_config_t,
        esp_lcd_rgb_panel_config_t__bindgen_ty_1, esp_lcd_rgb_timing_t,
        esp_lcd_rgb_timing_t__bindgen_ty_1, soc_periph_lcd_clk_src_t_LCD_CLK_SRC_PLL160M,
    },
};
use log::info;

mod ble;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();

    let panel_config = Arc::new(esp_lcd_rgb_panel_config_t {
        clk_src: soc_periph_lcd_clk_src_t_LCD_CLK_SRC_PLL160M,
        timings: esp_lcd_rgb_timing_t {
            pclk_hz: (16 * 1000 * 1000),
            h_res: 800,
            v_res: 480,
            hsync_pulse_width: 4,
            hsync_back_porch: 8,
            hsync_front_porch: 8,
            vsync_pulse_width: 4,
            vsync_back_porch: 8,
            vsync_front_porch: 8,
            flags: {
                let mut timings_flags = esp_lcd_rgb_timing_t__bindgen_ty_1::default();
                timings_flags.set_de_idle_high(0);
                timings_flags.set_pclk_active_neg(1);
                timings_flags.set_pclk_idle_high(0);
                timings_flags
            },
        },
        data_width: 16,
        bits_per_pixel: 0,
        num_fbs: 0,
        bounce_buffer_size_px: 0,
        sram_trans_align: 8,
        psram_trans_align: 64,
        hsync_gpio_num: 39,
        vsync_gpio_num: 41,
        de_gpio_num: 40,
        pclk_gpio_num: 42,
        disp_gpio_num: -1,
        data_gpio_nums: [8, 3, 46, 9, 1, 5, 6, 7, 15, 16, 4, 45, 48, 47, 21, 14],
        flags: {
            let mut panel_flags = esp_lcd_rgb_panel_config_t__bindgen_ty_1::default();
            panel_flags.set_disp_active_low(0);
            panel_flags.set_fb_in_psram(1);
            panel_flags
        },
    });

    let config = config::TimerConfig::new().frequency(25.kHz().into());
    let timer = LedcTimerDriver::new(peripherals.ledc.timer0, &config).unwrap();
    let mut backlight_pwm =
        LedcDriver::new(peripherals.ledc.channel0, timer, peripherals.pins.gpio2).unwrap();

    backlight_pwm
        .set_duty(backlight_pwm.get_max_duty() / 2)
        .unwrap();

    info!("Starting fb writer thread");
    let (send, receive) = std::sync::mpsc::channel();

    ThreadSpawnConfiguration {
        name: Some(b"fb writer\0"),
        pin_to_core: Some(esp_idf_svc::hal::cpu::Core::Core1),
        ..Default::default()
    }
    .set()
    .unwrap();

    let mutex = Arc::new(Mutex::new(true));
    let mutex2 = mutex.clone();
    std::thread::spawn(move || {
        let ret_panel = prepare_lcd_panel(&panel_config);

        loop {
            let ptr = receive.recv().unwrap();
            unsafe {
                let _lock = mutex2.lock().unwrap();
                esp_lcd_panel_draw_bitmap(ret_panel, 0, 0, 800, 480, ptr as *mut std::ffi::c_void);
            }
        }
    });
    ThreadSpawnConfiguration::default().set().unwrap();

    let (sender, receiver) = std::sync::mpsc::sync_channel(10);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let ble_device = BLEDevice::take();
            let mut ble_client = ble_device.new_client();

            ble_client.on_disconnect(|client| {
                panic!("Disconnected {:?}", client);
            });

            let mut ble_handler = RadarBle::new(ble_device, &mut ble_client).await.unwrap();

            ble_handler.notify_data(sender).await.unwrap();

            info!("Creating framebuffer");
            let mut raw_framebuffer_0 = vec![0u16; 800 * 480];
            let mut raw_framebuffer_1 = vec![0u16; 800 * 480];

            let mut fbuf0 = embedded_gfx::framebuffer::DmaReadyFramebuffer::<800, 480>::new(
                raw_framebuffer_0.as_mut_ptr() as *mut std::ffi::c_void,
                false,
            );
            let mut fbuf1 = embedded_gfx::framebuffer::DmaReadyFramebuffer::<800, 480>::new(
                raw_framebuffer_1.as_mut_ptr() as *mut std::ffi::c_void,
                false,
            );

            info!("Framebuffer created");

            let mut counter = 0;

            info!("Starting main loop");

            loop {
                let fbuf = if counter % 2 == 0 {
                    &mut fbuf0
                } else {
                    &mut fbuf1
                };

                fbuf.clear(Rgb565::new(42 >> 3, 83 >> 2, 87 >> 3)).unwrap();

                // Draw static radar background circles
                let center_x = 400;
                let center_y = 0; // Top of screen
                let max_radius = 480; // Full screen height

                // Draw concentric circles from largest to smallest
                for i in 0..5 {
                    let radius = max_radius - (i * (max_radius / 5));
                    // Convert center coordinates to top-left coordinates
                    let top_left_x = center_x - radius as i32;
                    let top_left_y = center_y - radius as i32;
                    // circle anchor is top-left corner
                    Circle::new(Point::new(top_left_x, top_left_y), radius * 2)
                        .into_styled(PrimitiveStyle::with_stroke(
                            Rgb565::new(108 >> 3, 208 >> 2, 200 >> 3),
                            2,
                        ))
                        .draw(fbuf)
                        .unwrap();
                }

                info!("Drawing");

                for target in receiver.try_iter() {
                    let gain = 480.0 / 5000.0;
                    // y parte da zero se sei davanti, va verso l'infinito
                    // x è negativo se sei a dx del modulo, positivo se sei a sx
                    let x = (target.position.x as f64 * gain) as i32 + center_x;
                    let y = (target.position.y as f64 * gain) as i32 + center_y;
                    Circle::new(Point::new(x, y), 20)
                        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(255, 0, 0)))
                        .draw(fbuf)
                        .unwrap();
                }

                let lock = mutex.lock().unwrap();
                std::mem::drop(lock);
                send.send(fbuf.as_mut_ptr() as *const () as usize).unwrap();

                counter += 1;
            }
        });
}

fn prepare_lcd_panel(panel_config: &esp_lcd_rgb_panel_config_t) -> esp_lcd_panel_handle_t {
    let mut esp_lcd_panel = esp_lcd_panel_t::default();
    let mut ret_panel = &mut esp_lcd_panel as *mut esp_idf_svc::sys::esp_lcd_panel_t;

    unsafe { esp_idf_svc::sys::esp_lcd_new_rgb_panel(panel_config, &mut ret_panel) };
    unsafe { esp_idf_svc::sys::esp_lcd_panel_reset(ret_panel) };
    unsafe { esp_idf_svc::sys::esp_lcd_panel_init(ret_panel) };
    unsafe { esp_lcd_panel_disp_on_off(ret_panel, true) };

    ret_panel
}
