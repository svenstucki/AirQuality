use esp_hal::{
    Blocking,
    gpio::Level,
    rmt::{Channel, PulseCode, Tx},
};

pub struct Led<'d> {
    channel: Option<Channel<'d, Blocking, Tx>>,
    buffer: [PulseCode; 25],
}

impl<'d> Led<'d> {
    pub fn new(channel: Channel<'d, Blocking, Tx>) -> Self {
        Self {
            channel: Some(channel),
            buffer: [PulseCode::default(); 25],
        }
    }

    pub fn write_rgb(&mut self, rgb: [u8; 3]) -> Result<(), esp_hal::rmt::Error> {
        let mut index = 0;
        for value in [rgb[1], rgb[0], rgb[2]] {
            for bit in [128, 64, 32, 16, 8, 4, 2, 1] {
                self.buffer[index] = if value & bit == 0 {
                    PulseCode::new(Level::High, 32, Level::Low, 68)
                } else {
                    PulseCode::new(Level::High, 68, Level::Low, 32)
                };
                index += 1;
            }
        }
        self.buffer[24] = PulseCode::end_marker();
        let channel = self.channel.take().unwrap();
        match channel.transmit(&self.buffer) {
            Ok(transaction) => match transaction.wait() {
                Ok(channel) => {
                    self.channel = Some(channel);
                    Ok(())
                }
                Err((error, channel)) => {
                    self.channel = Some(channel);
                    Err(error)
                }
            },
            Err((error, channel)) => {
                self.channel = Some(channel);
                Err(error)
            }
        }
    }
}
