//! Small, clock-injected countdowns. The UI owns wake-ups and notifications.

use std::fmt;
use std::time::{Duration, Instant};

pub const MAX_TIMERS: usize = 8;
pub const MAX_DURATION: Duration = Duration::from_secs(24 * 60 * 60);
pub type TimerId = u64;

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, PartialEq, Eq)]
pub struct TimerSpec {
    pub duration: Duration,
    pub label: String,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    InvalidSyntax,
    Zero,
    TooLong,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&crate::i18n::tr(match self {
            Self::Empty => "Indique une durée, par exemple 5m ou 1h 30m.",
            Self::InvalidSyntax => "Durée invalide. Exemples : 5m, 90s, 01:30 ou 1h 30m.",
            Self::Zero => "La durée doit être supérieure à zéro.",
            Self::TooLong => "La durée maximale est de 24 heures.",
        }))
    }
}

/// Parses a duration followed by an optional label, without starting a timer.
/// Bare integers mean minutes; a following numeric component needs a unit.
/// A label must be separated from the duration by whitespace.
pub fn parse(input: &str) -> Result<TimerSpec, ParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    if input
        .chars()
        .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(ParseError::InvalidSyntax);
    }
    let first_end = input.find(char::is_whitespace).unwrap_or(input.len());
    let first = &input[..first_end];
    if first.contains(':') {
        let seconds = parse_clock(first)?;
        let label = input[first_end..].trim();
        reject_numeric_label(label)?;
        return specification(seconds, label);
    }

    let mut position = 0;
    let mut seconds = 0_u64;
    let mut components = 0;
    loop {
        let number_start = position;
        while input
            .as_bytes()
            .get(position)
            .is_some_and(u8::is_ascii_digit)
        {
            position += 1;
        }
        if position == number_start {
            return Err(ParseError::InvalidSyntax);
        }
        let number = input[number_start..position]
            .parse::<u64>()
            .map_err(|_| ParseError::TooLong)?;
        let after_number = position;
        position = skip_whitespace(input, position);
        let unit_start = position;
        while input
            .as_bytes()
            .get(position)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            position += 1;
        }
        let word = &input[unit_start..position];
        let multiplier = unit_multiplier(word);
        if let Some(multiplier) = multiplier {
            let part = number.checked_mul(multiplier).ok_or(ParseError::TooLong)?;
            seconds = seconds.checked_add(part).ok_or(ParseError::TooLong)?;
            if seconds > MAX_DURATION.as_secs() {
                return Err(ParseError::TooLong);
            }
            components += 1;
        } else {
            // Attached unknown units are always a typo, never a label. After a
            // unit-bearing component a bare number is also ambiguous.
            let separated = after_number != unit_start;
            if !word.is_empty() && !separated || components != 0 {
                return Err(ParseError::InvalidSyntax);
            }
            let label = &input[after_number..];
            if !label.is_empty() && !label.starts_with(char::is_whitespace) {
                return Err(ParseError::InvalidSyntax);
            }
            let label = label.trim();
            reject_numeric_label(label)?;
            let seconds = number.checked_mul(60).ok_or(ParseError::TooLong)?;
            return specification(seconds, label);
        }

        let after_unit = position;
        position = skip_whitespace(input, position);
        if position == input.len() {
            return specification(seconds, "");
        }
        if input.as_bytes()[position].is_ascii_digit() {
            continue;
        }
        // Accept compact compounds such as 1h30m, but never an attached label.
        if after_unit == position {
            return Err(ParseError::InvalidSyntax);
        }
        let label = &input[position..];
        reject_numeric_label(label)?;
        return specification(seconds, label);
    }
}

fn skip_whitespace(input: &str, mut position: usize) -> usize {
    while let Some(character) = input[position..].chars().next() {
        if !character.is_whitespace() {
            break;
        }
        position += character.len_utf8();
    }
    position
}

fn unit_multiplier(unit: &str) -> Option<u64> {
    match unit.to_ascii_lowercase().as_str() {
        "h" | "hr" | "hrs" | "hour" | "hours" | "heure" | "heures" => Some(3600),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60),
        "s" | "sec" | "secs" | "second" | "seconds" | "seconde" | "secondes" => Some(1),
        _ => None,
    }
}

fn reject_numeric_label(label: &str) -> Result<(), ParseError> {
    if label.starts_with(|character: char| {
        character.is_ascii_digit() || matches!(character, '-' | '+' | '.' | ':' | ',')
    }) {
        Err(ParseError::InvalidSyntax)
    } else {
        Ok(())
    }
}

fn parse_clock(clock: &str) -> Result<u64, ParseError> {
    let parts: Vec<&str> = clock.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return Err(ParseError::InvalidSyntax);
    }
    let mut values = [0_u64; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ParseError::InvalidSyntax);
        }
        values[index] = part.parse::<u64>().map_err(|_| ParseError::TooLong)?;
    }
    if values[parts.len() - 1] >= 60 || parts.len() == 3 && values[1] >= 60 {
        return Err(ParseError::InvalidSyntax);
    }
    let seconds = if parts.len() == 2 {
        values[0]
            .checked_mul(60)
            .and_then(|minutes| minutes.checked_add(values[1]))
    } else {
        values[0]
            .checked_mul(3600)
            .and_then(|hours| {
                values[1]
                    .checked_mul(60)
                    .and_then(|minutes| hours.checked_add(minutes))
            })
            .and_then(|minutes| minutes.checked_add(values[2]))
    };
    seconds.ok_or(ParseError::TooLong)
}

fn specification(seconds: u64, label: &str) -> Result<TimerSpec, ParseError> {
    if seconds == 0 {
        return Err(ParseError::Zero);
    }
    if seconds > MAX_DURATION.as_secs() {
        return Err(ParseError::TooLong);
    }
    Ok(TimerSpec {
        duration: Duration::from_secs(seconds),
        label: if label.is_empty() {
            crate::i18n::tr("Minuteur")
        } else {
            label.to_owned()
        },
    })
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimerStatus {
    Running,
    Paused,
    Finished,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, PartialEq, Eq)]
pub struct TimerSnapshot {
    pub id: TimerId,
    pub label: String,
    pub status: TimerStatus,
    pub remaining_seconds: u64,
    pub total_seconds: u64,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, PartialEq, Eq)]
pub struct TimerAlert {
    pub id: TimerId,
    pub label: String,
}

#[cfg_attr(test, derive(Debug))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TimerError {
    Capacity,
    InvalidDuration,
    UnknownTimer,
    NotRunning,
    NotPaused,
    NotFinished,
    IdExhausted,
}

impl fmt::Display for TimerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&crate::i18n::tr(match self {
            Self::Capacity => "Huit minuteurs maximum. Supprime un minuteur pour en ajouter un.",
            Self::InvalidDuration => "La durée doit être comprise entre une seconde et 24 heures.",
            Self::UnknownTimer => "Ce minuteur n'existe plus.",
            Self::NotRunning => "Ce minuteur ne tourne pas.",
            Self::NotPaused => "Ce minuteur n'est pas en pause.",
            Self::NotFinished => "Ce minuteur n'est pas terminé.",
            Self::IdExhausted => "Impossible de créer un nouvel identifiant de minuteur.",
        }))
    }
}

#[cfg_attr(test, derive(Debug))]
enum State {
    Running { deadline: Instant },
    Paused { remaining: Duration },
    Finished { notified: bool },
}

#[cfg_attr(test, derive(Debug))]
struct Timer {
    id: TimerId,
    label: String,
    total: Duration,
    state: State,
}

impl Timer {
    fn refresh(&mut self, now: Instant) {
        if matches!(self.state, State::Running { deadline } if now >= deadline) {
            self.state = State::Finished { notified: false };
        }
    }

    fn snapshot(&self, now: Instant) -> TimerSnapshot {
        let (status, remaining) = match self.state {
            State::Running { deadline } if now < deadline => {
                (TimerStatus::Running, deadline.duration_since(now))
            }
            State::Running { .. } | State::Finished { .. } => {
                (TimerStatus::Finished, Duration::ZERO)
            }
            State::Paused { remaining } => (TimerStatus::Paused, remaining),
        };
        TimerSnapshot {
            id: self.id,
            label: self.label.clone(),
            status,
            remaining_seconds: rounded_seconds(remaining),
            total_seconds: rounded_seconds(self.total),
        }
    }
}

fn rounded_seconds(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}

/// Keeps at most eight countdowns, including completed ones awaiting dismissal.
/// No thread, polling, file access or notification is performed by this type.
#[cfg_attr(test, derive(Debug))]
pub struct TimerBook {
    timers: Vec<Timer>,
    next_id: TimerId,
}

impl Default for TimerBook {
    fn default() -> Self {
        Self {
            timers: Vec::new(),
            next_id: 1,
        }
    }
}

impl TimerBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self, specification: TimerSpec, now: Instant) -> Result<TimerId, TimerError> {
        if specification.duration < Duration::from_secs(1) || specification.duration > MAX_DURATION
        {
            return Err(TimerError::InvalidDuration);
        }
        if self.timers.len() >= MAX_TIMERS {
            return Err(TimerError::Capacity);
        }
        let deadline = now
            .checked_add(specification.duration)
            .ok_or(TimerError::InvalidDuration)?;
        let id = self.next_id;
        let next_id = id.checked_add(1).ok_or(TimerError::IdExhausted)?;
        self.timers.push(Timer {
            id,
            label: specification.label,
            total: specification.duration,
            state: State::Running { deadline },
        });
        self.next_id = next_id;
        Ok(id)
    }

    /// Converts elapsed deadlines to completed timers and drains each alert once.
    /// Calling it after the menu was hidden or the computer slept catches up.
    pub fn update(&mut self, now: Instant) -> Vec<TimerAlert> {
        let mut alerts = Vec::new();
        for timer in &mut self.timers {
            timer.refresh(now);
            if let State::Finished { notified } = &mut timer.state {
                if !*notified {
                    *notified = true;
                    alerts.push(TimerAlert {
                        id: timer.id,
                        label: timer.label.clone(),
                    });
                }
            }
        }
        alerts
    }

    /// The UI can stop its wake-up timer when all countdowns are paused or done.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers
            .iter()
            .filter_map(|timer| match timer.state {
                State::Running { deadline } => Some(deadline),
                _ => None,
            })
            .min()
    }

    pub fn snapshots(&self, now: Instant) -> Vec<TimerSnapshot> {
        self.timers
            .iter()
            .map(|timer| timer.snapshot(now))
            .collect()
    }

    pub fn pause(&mut self, id: TimerId, now: Instant) -> Result<(), TimerError> {
        let timer = self.find_mut(id)?;
        timer.refresh(now);
        if let State::Running { deadline } = timer.state {
            timer.state = State::Paused {
                remaining: deadline.duration_since(now),
            };
            Ok(())
        } else {
            Err(TimerError::NotRunning)
        }
    }

    pub fn resume(&mut self, id: TimerId, now: Instant) -> Result<(), TimerError> {
        let timer = self.find_mut(id)?;
        if let State::Paused { remaining } = timer.state {
            let deadline = now
                .checked_add(remaining)
                .ok_or(TimerError::InvalidDuration)?;
            timer.state = State::Running { deadline };
            Ok(())
        } else {
            Err(TimerError::NotPaused)
        }
    }

    pub fn cancel(&mut self, id: TimerId) -> Result<(), TimerError> {
        let position = self
            .timers
            .iter()
            .position(|timer| timer.id == id)
            .ok_or(TimerError::UnknownTimer)?;
        self.timers.remove(position);
        Ok(())
    }

    pub fn acknowledge(&mut self, id: TimerId, now: Instant) -> Result<(), TimerError> {
        let timer = self.find_mut(id)?;
        timer.refresh(now);
        if !matches!(timer.state, State::Finished { .. }) {
            return Err(TimerError::NotFinished);
        }
        self.cancel(id)
    }

    fn find_mut(&mut self, id: TimerId) -> Result<&mut Timer, TimerError> {
        self.timers
            .iter_mut()
            .find(|timer| timer.id == id)
            .ok_or(TimerError::UnknownTimer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_durations_and_optional_labels() {
        for (input, seconds, label) in [
            ("5m", 300, "Minuteur"),
            ("10 min", 600, "Minuteur"),
            ("1h 30m", 5400, "Minuteur"),
            ("90s", 90, "Minuteur"),
            ("01:30", 90, "Minuteur"),
            ("1:02:30", 3750, "Minuteur"),
            ("5", 300, "Minuteur"),
            (" 5m Thé ", 300, "Thé"),
            ("10 min Pause café", 600, "Pause café"),
            ("2 Thé", 120, "Thé"),
            ("1h30m", 5400, "Minuteur"),
            ("1 HEURE 2 minutes 3 secondes Repas", 3723, "Repas"),
            ("1 HOUR 2 minutes 3 seconds Break", 3723, "Break"),
            ("2 hours Workout", 7200, "Workout"),
            ("1 second Tea", 1, "Tea"),
            ("24h", 86400, "Minuteur"),
            ("1440:00", 86400, "Minuteur"),
            ("24:00:00", 86400, "Minuteur"),
            ("5\u{a0}min Thé", 300, "Thé"),
        ] {
            let specification = parse(input).unwrap_or_else(|error| panic!("{input}: {error}"));
            assert_eq!(specification.duration.as_secs(), seconds, "{input}");
            assert_eq!(specification.label, label, "{input}");
        }
    }

    #[test]
    fn rejects_zero_overflow_and_durations_above_one_day() {
        for input in ["0", "0m", "00:00", "0h 0m 0s"] {
            assert_eq!(parse(input), Err(ParseError::Zero), "{input}");
        }
        for input in [
            "24h 1s",
            "1441",
            "24:00:01",
            "18446744073709551615h",
            "18446744073709551616s",
        ] {
            assert_eq!(parse(input), Err(ParseError::TooLong), "{input}");
        }
    }

    #[test]
    fn rejects_ambiguous_or_malformed_input_instead_of_guessing() {
        assert_eq!(parse(" "), Err(ParseError::Empty));
        for input in [
            "-5m",
            "+5m",
            "1.5m",
            "1,5 min",
            "5x",
            "10minx",
            "5mThé",
            "1h 30",
            "5m 2x",
            "5m -2s",
            "5m 1.5s",
            "01:60",
            "1:60:00",
            "1::30",
            ":30",
            "1:2:3:4",
            "1:30m",
            "1:30 5m",
            "5m\0 Thé",
            "5m :30",
        ] {
            assert_eq!(parse(input), Err(ParseError::InvalidSyntax), "{input}");
        }
    }

    #[test]
    fn countdown_rounds_up_and_delivers_expiration_once_until_acknowledged() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        let id = timers.start(parse("5s Thé").unwrap(), now).unwrap();
        assert_eq!(
            timers.snapshots(now + Duration::from_millis(1))[0].remaining_seconds,
            5
        );
        assert_eq!(
            timers.snapshots(now + Duration::from_millis(4001))[0].remaining_seconds,
            1
        );
        assert!(timers.update(now + Duration::from_millis(4999)).is_empty());
        assert_eq!(
            timers.update(now + Duration::from_secs(5)),
            vec![TimerAlert {
                id,
                label: "Thé".to_owned()
            }]
        );
        assert!(timers.update(now + Duration::from_secs(10)).is_empty());
        let snapshot = &timers.snapshots(now + Duration::from_secs(10))[0];
        assert_eq!(snapshot.status, TimerStatus::Finished);
        assert_eq!(snapshot.remaining_seconds, 0);
        assert_eq!(snapshot.total_seconds, 5);
        timers
            .acknowledge(id, now + Duration::from_secs(10))
            .unwrap();
        assert!(timers.snapshots(now).is_empty());
    }

    #[test]
    fn pause_preserves_fractional_time_and_resume_uses_a_new_deadline() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        let id = timers.start(parse("10s").unwrap(), now).unwrap();
        timers.pause(id, now + Duration::from_millis(2250)).unwrap();
        let snapshot = &timers.snapshots(now + Duration::from_secs(100))[0];
        assert_eq!(snapshot.status, TimerStatus::Paused);
        assert_eq!(snapshot.remaining_seconds, 8);
        assert!(timers.update(now + Duration::from_secs(100)).is_empty());
        timers.resume(id, now + Duration::from_secs(100)).unwrap();
        assert!(timers
            .update(now + Duration::from_millis(107749))
            .is_empty());
        assert_eq!(timers.update(now + Duration::from_millis(107750)).len(), 1);
    }

    #[test]
    fn overdue_pause_cannot_hide_the_completion_alert() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        let id = timers.start(parse("1s").unwrap(), now).unwrap();
        assert_eq!(
            timers.pause(id, now + Duration::from_secs(2)),
            Err(TimerError::NotRunning)
        );
        assert_eq!(timers.update(now + Duration::from_secs(3)).len(), 1);
        assert!(timers.update(now + Duration::from_secs(4)).is_empty());
        assert_eq!(timers.resume(id, now), Err(TimerError::NotPaused));
    }

    #[test]
    fn long_periods_without_updates_count_and_finish_all_due_timers() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        let first = timers.start(parse("1m Court").unwrap(), now).unwrap();
        let second = timers.start(parse("2h Long").unwrap(), now).unwrap();
        let alerts = timers.update(now + Duration::from_secs(3 * 3600));
        assert_eq!(
            alerts.iter().map(|alert| alert.id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(timers
            .snapshots(now + Duration::from_secs(3 * 3600))
            .iter()
            .all(|timer| timer.status == TimerStatus::Finished));
        assert!(timers
            .update(now + Duration::from_secs(4 * 3600))
            .is_empty());
    }

    #[test]
    fn capacity_includes_finished_timers_and_ids_are_never_reused() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        for expected in 1..=MAX_TIMERS as u64 {
            assert_eq!(timers.start(parse("1s").unwrap(), now), Ok(expected));
        }
        assert_eq!(
            timers.start(parse("1s").unwrap(), now),
            Err(TimerError::Capacity)
        );
        timers.update(now + Duration::from_secs(1));
        assert_eq!(
            timers.start(parse("1s").unwrap(), now),
            Err(TimerError::Capacity)
        );
        timers.acknowledge(3, now + Duration::from_secs(1)).unwrap();
        assert_eq!(timers.start(parse("1s").unwrap(), now), Ok(9));
        timers.cancel(1).unwrap();
        assert_eq!(timers.start(parse("1s").unwrap(), now), Ok(10));
    }

    #[test]
    fn next_deadline_tracks_only_running_countdowns() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        assert_eq!(timers.next_deadline(), None);
        let first = timers.start(parse("3s").unwrap(), now).unwrap();
        let second = timers.start(parse("5s").unwrap(), now).unwrap();
        assert_eq!(timers.next_deadline(), Some(now + Duration::from_secs(3)));
        timers.pause(first, now + Duration::from_secs(1)).unwrap();
        assert_eq!(timers.next_deadline(), Some(now + Duration::from_secs(5)));
        timers.pause(second, now + Duration::from_secs(1)).unwrap();
        assert_eq!(timers.next_deadline(), None);
        timers.resume(first, now + Duration::from_secs(10)).unwrap();
        assert_eq!(timers.next_deadline(), Some(now + Duration::from_secs(12)));
        timers.update(now + Duration::from_secs(12));
        assert_eq!(timers.next_deadline(), None);
    }

    #[test]
    fn cancellation_and_invalid_operations_do_not_create_alerts() {
        let now = Instant::now();
        let mut timers = TimerBook::new();
        let id = timers.start(parse("1s").unwrap(), now).unwrap();
        assert_eq!(timers.resume(id, now), Err(TimerError::NotPaused));
        assert_eq!(timers.acknowledge(id, now), Err(TimerError::NotFinished));
        assert_eq!(timers.pause(42, now), Err(TimerError::UnknownTimer));
        timers.cancel(id).unwrap();
        assert!(timers.update(now + Duration::from_secs(10)).is_empty());
        assert_eq!(timers.cancel(id), Err(TimerError::UnknownTimer));
        assert_eq!(
            timers.start(
                TimerSpec {
                    duration: Duration::ZERO,
                    label: "Invalid".to_owned()
                },
                now
            ),
            Err(TimerError::InvalidDuration)
        );
        assert_eq!(
            timers.start(
                TimerSpec {
                    duration: MAX_DURATION + Duration::from_secs(1),
                    label: "Invalid".to_owned()
                },
                now
            ),
            Err(TimerError::InvalidDuration)
        );
    }
}
