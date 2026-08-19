#[cfg(not(test))]
use embedded_hal_async::i2c::I2c;

pub const ADDRESS: u8 = 0x68;

pub const CLOCK_VALID: u8 = 1 << 0;
pub const RTC_VALID: u8 = 1 << 1;
pub const RTC_OSF: u8 = 1 << 2;
pub const RTC_READ_ERROR: u8 = 1 << 3;

#[cfg(not(test))]
const CONTROL_REGISTER: u8 = 0x0e;
#[cfg(not(test))]
const STATUS_REGISTER: u8 = 0x0f;
#[cfg(not(test))]
const OSF: u8 = 1 << 7;
#[cfg(not(test))]
const EOSC: u8 = 1 << 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    I2c,
    InvalidBcd,
    InvalidDate,
    Verification,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DateTime {
    year: u16,
    month: u8,
    day: u8,
    weekday: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub unix_seconds: u32,
    pub oscillator_stopped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockState {
    pub unix_at_anchor: u32,
    pub monotonic_at_anchor: u64,
    pub local_clock_valid: bool,
    pub rtc_valid: bool,
    pub oscillator_stopped: bool,
    pub read_error: bool,
}

impl Default for ClockState {
    fn default() -> Self {
        Self {
            unix_at_anchor: 0,
            monotonic_at_anchor: 0,
            local_clock_valid: false,
            rtc_valid: false,
            oscillator_stopped: false,
            read_error: true,
        }
    }
}

impl ClockState {
    pub fn anchored(unix_seconds: u32, monotonic_seconds: u64) -> Self {
        Self {
            unix_at_anchor: unix_seconds,
            monotonic_at_anchor: monotonic_seconds,
            local_clock_valid: true,
            rtc_valid: true,
            oscillator_stopped: false,
            read_error: false,
        }
    }

    pub fn current_unix(&self, monotonic_seconds: u64) -> Option<u32> {
        if !self.local_clock_valid {
            return None;
        }
        Some(
            self.unix_at_anchor
                .saturating_add(monotonic_seconds.saturating_sub(self.monotonic_at_anchor) as u32),
        )
    }

    pub fn flags(&self) -> u8 {
        (if self.local_clock_valid {
            CLOCK_VALID
        } else {
            0
        }) | (if self.rtc_valid { RTC_VALID } else { 0 })
            | (if self.oscillator_stopped { RTC_OSF } else { 0 })
            | (if self.read_error { RTC_READ_ERROR } else { 0 })
    }

    pub fn payload(&self, monotonic_seconds: u64) -> [u8; 5] {
        let mut payload = [0u8; 5];
        if let Some(unix_seconds) = self.current_unix(monotonic_seconds) {
            payload[..4].copy_from_slice(&unix_seconds.to_le_bytes());
        }
        payload[4] = self.flags();
        payload
    }
}

#[derive(Clone, Copy)]
pub enum Command {
    SetUnix(u32),
}

#[derive(Clone, Copy)]
pub struct Response {
    pub success: bool,
    pub state: ClockState,
}

#[derive(Clone, Copy)]
pub enum ClockEvent {
    StartupUnavailable,
    Anchored {
        unix_seconds: u32,
        monotonic_seconds: u64,
    },
}

pub struct Ds3231;

#[cfg(not(test))]
impl Ds3231 {
    pub const fn new() -> Self {
        Self
    }

    pub async fn initialize<I2C>(&mut self, i2c: &mut I2C) -> Result<(), Error>
    where
        I2C: I2c,
    {
        let mut control = [0u8; 1];
        i2c.write_read(ADDRESS, &[CONTROL_REGISTER], &mut control)
            .await
            .map_err(|_| Error::I2c)?;
        if control[0] & EOSC != 0 {
            i2c.write(ADDRESS, &[CONTROL_REGISTER, control[0] & !EOSC])
                .await
                .map_err(|_| Error::I2c)?;
        }
        Ok(())
    }

    pub async fn read<I2C>(&mut self, i2c: &mut I2C) -> Result<Sample, Error>
    where
        I2C: I2c,
    {
        let mut time = [0u8; 7];
        i2c.write_read(ADDRESS, &[0x00], &mut time)
            .await
            .map_err(|_| Error::I2c)?;
        let date_time = decode_datetime(&time)?;

        let mut status = [0u8; 1];
        i2c.write_read(ADDRESS, &[STATUS_REGISTER], &mut status)
            .await
            .map_err(|_| Error::I2c)?;

        Ok(Sample {
            unix_seconds: datetime_to_unix(date_time)?,
            oscillator_stopped: status[0] & OSF != 0,
        })
    }

    pub async fn set_unix<I2C>(&mut self, i2c: &mut I2C, unix_seconds: u32) -> Result<Sample, Error>
    where
        I2C: I2c,
    {
        let date_time = unix_to_datetime(unix_seconds)?;
        let mut write = [0u8; 8];
        write[0] = 0x00;
        write[1] = to_bcd(date_time.second);
        write[2] = to_bcd(date_time.minute);
        write[3] = to_bcd(date_time.hour);
        write[4] = to_bcd(date_time.weekday);
        write[5] = to_bcd(date_time.day);
        write[6] = to_bcd(date_time.month) | if date_time.year >= 2100 { 0x80 } else { 0 };
        write[7] = to_bcd((date_time.year % 100) as u8);
        i2c.write(ADDRESS, &write).await.map_err(|_| Error::I2c)?;

        let mut status = [0u8; 1];
        i2c.write_read(ADDRESS, &[STATUS_REGISTER], &mut status)
            .await
            .map_err(|_| Error::I2c)?;
        i2c.write(ADDRESS, &[STATUS_REGISTER, status[0] & !OSF])
            .await
            .map_err(|_| Error::I2c)?;

        let sample = self.read(i2c).await?;
        if sample.oscillator_stopped || sample.unix_seconds.abs_diff(unix_seconds) > 1 {
            return Err(Error::Verification);
        }
        Ok(sample)
    }
}

fn decode_datetime(bytes: &[u8; 7]) -> Result<DateTime, Error> {
    let second = from_bcd(bytes[0] & 0x7f)?;
    let minute = from_bcd(bytes[1] & 0x7f)?;
    let hour = if bytes[2] & 0x40 != 0 {
        let hour12 = from_bcd(bytes[2] & 0x1f)?;
        if !(1..=12).contains(&hour12) {
            return Err(Error::InvalidDate);
        }
        if bytes[2] & 0x20 != 0 {
            if hour12 == 12 { 12 } else { hour12 + 12 }
        } else if hour12 == 12 {
            0
        } else {
            hour12
        }
    } else {
        from_bcd(bytes[2] & 0x3f)?
    };
    let weekday = from_bcd(bytes[3] & 0x07)?;
    let day = from_bcd(bytes[4] & 0x3f)?;
    let month = from_bcd(bytes[5] & 0x1f)?;
    let year = 2000 + from_bcd(bytes[6])? as u16 + if bytes[5] & 0x80 != 0 { 100 } else { 0 };
    let date_time = DateTime {
        year,
        month,
        day,
        weekday,
        hour,
        minute,
        second,
    };
    validate_datetime(date_time)?;
    Ok(date_time)
}

fn validate_datetime(date_time: DateTime) -> Result<(), Error> {
    if !(2000..=2106).contains(&date_time.year)
        || !(1..=12).contains(&date_time.month)
        || !(1..=days_in_month(date_time.year, date_time.month)).contains(&date_time.day)
        || date_time.weekday == 0
        || date_time.weekday > 7
        || date_time.hour > 23
        || date_time.minute > 59
        || date_time.second > 59
    {
        return Err(Error::InvalidDate);
    }
    Ok(())
}

fn datetime_to_unix(date_time: DateTime) -> Result<u32, Error> {
    validate_datetime(date_time)?;
    let mut days = 0u32;
    for year in 1970..date_time.year {
        days += if is_leap_year(year) { 366 } else { 365 };
    }
    for month in 1..date_time.month {
        days += days_in_month(date_time.year, month) as u32;
    }
    days += (date_time.day - 1) as u32;
    days.checked_mul(86_400)
        .and_then(|value| value.checked_add(date_time.hour as u32 * 3_600))
        .and_then(|value| value.checked_add(date_time.minute as u32 * 60))
        .and_then(|value| value.checked_add(date_time.second as u32))
        .ok_or(Error::InvalidDate)
}

fn unix_to_datetime(unix_seconds: u32) -> Result<DateTime, Error> {
    let mut days = unix_seconds / 86_400;
    let day_seconds = unix_seconds % 86_400;
    let mut year = 1970u16;
    while days >= if is_leap_year(year) { 366 } else { 365 } {
        days -= if is_leap_year(year) { 366 } else { 365 };
        year += 1;
    }
    let mut month = 1u8;
    while days >= days_in_month(year, month) as u32 {
        days -= days_in_month(year, month) as u32;
        month += 1;
    }
    let date_time = DateTime {
        year,
        month,
        day: days as u8 + 1,
        weekday: ((unix_seconds / 86_400 + 4) % 7 + 1) as u8,
        hour: (day_seconds / 3_600) as u8,
        minute: ((day_seconds % 3_600) / 60) as u8,
        second: (day_seconds % 60) as u8,
    };
    validate_datetime(date_time)?;
    Ok(date_time)
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn from_bcd(value: u8) -> Result<u8, Error> {
    let low = value & 0x0f;
    let high = value >> 4;
    if low > 9 || high > 9 {
        return Err(Error::InvalidBcd);
    }
    Ok(high * 10 + low)
}

#[cfg(not(test))]
fn to_bcd(value: u8) -> u8 {
    (value / 10) << 4 | value % 10
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_datetime() -> DateTime {
        DateTime {
            year: 2024,
            month: 2,
            day: 29,
            weekday: 5,
            hour: 12,
            minute: 34,
            second: 56,
        }
    }

    #[test]
    fn round_trip_leap_day() {
        let date_time = sample_datetime();
        let unix = datetime_to_unix(date_time).unwrap();
        assert_eq!(unix_to_datetime(unix).unwrap(), date_time);
    }

    #[test]
    fn decodes_24_hour_bcd_time() {
        let bytes = [0x56, 0x34, 0x12, 0x05, 0x29, 0x02, 0x24];
        assert_eq!(decode_datetime(&bytes).unwrap(), sample_datetime());
    }

    #[test]
    fn decodes_12_hour_bcd_time() {
        let bytes = [0x00, 0x00, 0x72, 0x01, 0x01, 0x01, 0x24];
        assert_eq!(decode_datetime(&bytes).unwrap().hour, 12);
        let bytes = [0x00, 0x00, 0x52, 0x01, 0x01, 0x01, 0x24];
        assert_eq!(decode_datetime(&bytes).unwrap().hour, 0);
    }

    #[test]
    fn rejects_invalid_bcd_and_dates() {
        assert_eq!(from_bcd(0x1a), Err(Error::InvalidBcd));
        let bytes = [0x00, 0x00, 0x00, 0x01, 0x31, 0x02, 0x23];
        assert_eq!(decode_datetime(&bytes), Err(Error::InvalidDate));
    }

    #[test]
    fn payload_extrapolates_local_clock() {
        let state = ClockState::anchored(1_700_000_000, 100);
        let payload = state.payload(160);
        assert_eq!(
            u32::from_le_bytes(payload[..4].try_into().unwrap()),
            1_700_000_060
        );
        assert_eq!(payload[4], CLOCK_VALID | RTC_VALID);
    }
}
