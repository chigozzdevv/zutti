fn main() -> std::process::ExitCode {
    eprintln!(
        "source proving unavailable: full consensus transitions and genesis-prefix proof are not implemented"
    );
    std::process::ExitCode::from(2)
}
