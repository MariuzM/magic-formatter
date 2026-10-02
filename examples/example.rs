struct Config {
    name:    String,
    retries: u32,
}

fn create(a: i32, b: i32, c: i32) -> i32 {
    a + b + c
}

fn magic_trailing_comma() -> i32 {
    create(
        1,
        2,
        3,
    )
}

fn first_argument_on_next_line_expands() -> i32 {
    create(
        1,
        2,
        3,
    )
}

fn first_argument_on_same_line_collapses() -> i32 {
    create(1, 2, 3)
}

fn single_line_if(value: i32) -> i32 {
    if value == 0 { return 1; };
    if value > 10 {
        return 10;
    }
    value
}

fn method_chain_expands() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(4002)
}

fn method_chain_stays_inline() -> Option<u16> {
    std::env::var("PORT").ok().and_then(|p| p.parse::<u16>().ok())
}

fn aligned_assignments(config: &mut Config) -> u32 {
    config.name    = String::from("example");
    config.retries = 3;
    let total      = config.retries + 1;

    let unaligned_after_blank_line = total * 2;
    unaligned_after_blank_line
}

fn messy_spacing(a: i32, b: i32) -> i32 {
    let sum = a + b;
    sum * 2
}
