use crate::json as codec;

// Grammar and error ordering follow locked cron 3.1.6; see vendor/CRON-LICENSE.txt.
pub(crate) fn parse(expression: &str) -> Result<[u64; 6], String> {
    let lower = expression.to_lowercase();
    let source = match lower.as_str() {
        "@yearly" => "0 0 0 1 1 *",
        "@monthly" => "0 0 0 1 * *",
        "@weekly" => "0 0 0 * * 0",
        "@daily" => "0 0 0 * * *",
        "@hourly" => "0 0 * * * *",
        "@minutely" => "0 * * * * *",
        "@secondly" => "* * * * * *",
        "@weekdays" => "0 0 0 * * 1-5",
        "@weekends" => "0 0 0 * * 0,6",
        _ => &lower,
    };
    let mut expanded = String::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor].is_ascii_alphabetic() {
            let mut end = cursor;
            while end < source.len() && end < cursor + 3 && source.as_bytes()[end].is_ascii_alphabetic() {
                end += 1;
            }
            let alias = &source[cursor..end];
            let value = match alias {
                "jan" | "mon" => 1,
                "feb" | "tue" => 2,
                "mar" | "wed" => 3,
                "apr" | "thu" => 4,
                "may" | "fri" => 5,
                "jun" | "sat" => 6,
                "jul" => 7,
                "aug" => 8,
                "sep" => 9,
                "oct" => 10,
                "nov" => 11,
                "dec" => 12,
                "sun" => 0,
                _ => return Err(format!("Unknown alias: {alias}")),
            };
            expanded.push_str(&value.to_string());
            cursor = end;
        } else {
            let character = source[cursor..].chars().next().unwrap();
            expanded.push(character);
            cursor += character.len_utf8();
        }
    }
    let mut fields = expanded.split(codec::is_space).filter(|field| !field.is_empty()).collect::<Vec<_>>();
    if fields.len() < 5 {
        return Err("Too few fields".to_owned());
    }
    if fields.len() > 6 {
        return Err("Too many fields".to_owned());
    }
    if fields.len() == 5 {
        fields.insert(0, "0");
    }
    let units = [("second", 0, 59), ("minute", 0, 59), ("hour", 0, 23),
        ("dayOfMonth", 1, 31), ("month", 1, 12), ("dayOfWeek", 0, 7)];
    let mut masks = [0u64; 6];
    for (index, (unit, low, high)) in units.into_iter().enumerate() {
        let value = fields[index];
        for field in value.split(',') {
            if field.find('*').is_some_and(|position| position != 0) {
                return Err(format!("Field ({field}) has an invalid wildcard expression"));
            }
        }
        let value = value.replace('*', &format!("{low}-{high}"));
        for range in value.split(',') {
            let invalid = || format!("Field ({unit}) cannot be parsed");
            let (bounds, step) = range.split_once('/').map_or((range, None), |(bounds, step)| (bounds, Some(step)));
            let (start, end) = bounds.split_once('-').map_or((bounds, None), |(start, end)| (start, Some(end)));
            let numeric = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
            if !numeric(start) || end.is_some_and(|end| !numeric(end)) || step.is_some_and(|step| !numeric(step)) {
                return Err(invalid());
            }
            let number = |text: &str| text.parse::<f64>().unwrap_or(f64::INFINITY);
            let lower = number(start);
            let upper = end.map(number);
            let increment = step.map(number).unwrap_or(1.0);
            if increment == 0.0 {
                return Err(format!("Field ({unit}) has a step of zero"));
            }
            if upper.is_some_and(|upper| lower > upper) {
                return Err(format!("Field ({unit}) has an invalid range"));
            }
            if lower < f64::from(low) || upper.unwrap_or(lower) > f64::from(high) {
                return Err(format!("Field value ({value}) is out of range"));
            }
            let last = upper.unwrap_or(if step.is_some() { f64::from(high) } else { lower }) as u32;
            let mut pointer = lower as u32;
            while pointer <= last {
                masks[index] |= 1 << pointer;
                pointer += increment.min(64.0) as u32;
            }
        }
    }
    if masks[5] & (1 << 7) != 0 {
        masks[5] = (masks[5] & !(1 << 7)) | 1;
    }
    let month_days = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let months = (1..=12).filter(|month| masks[4] & (1 << month) != 0).collect::<Vec<_>>();
    if !months.iter().any(|month| (1..=month_days[month - 1]).any(|day| masks[3] & (1 << day) != 0)) {
        let limit = month_days[months[months.len() - 1] - 1];
        let mut fixed = 0;
        for day in 1..=31 {
            if masks[3] & (1 << day) != 0 {
                fixed |= 1 << (day % limit);
            }
        }
        masks[3] = fixed;
    }
    Ok(masks)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn locked_cron_calendar_and_rejections() {
        assert_eq!(parse("0 9 * JAN MON-FRI").unwrap(), [1, 1, 1 << 9, 0xffff_fffe, 2, 0b11_1110]);
        assert_eq!(parse("@weekends").unwrap()[5], 0b100_0001);
        assert_eq!(parse("0 0 0 31 feb *").unwrap()[3], 1 << 2);
        assert_eq!(parse("*/15 * * * * *").unwrap()[0], (1 << 45) | (1 << 30) | (1 << 15) | 1);
        for (value, reason) in [
            ("not-a-cron", "Unknown alias: not"),
            ("* * * *", "Too few fields"),
            ("0/0 * * * * *", "Field (second) has a step of zero"),
            ("0 0 0 32 * *", "Field value (32) is out of range"),
            ("0 0 0 * * 5-1", "Field (dayOfWeek) has an invalid range"),
            ("0* * * * * *", "Field (0*) has an invalid wildcard expression"),
        ] {
            assert_eq!(parse(value).unwrap_err(), reason);
        }
    }
}
