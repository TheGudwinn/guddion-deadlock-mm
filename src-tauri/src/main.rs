// No console window next to the app, in debug builds too.
#![windows_subsystem = "windows"]

fn main() {
    deadlock_mm_lib::run()
}
