use crate::session::Cleaner;

pub struct PassthroughCleaner;

impl Cleaner for PassthroughCleaner {
    fn clean(&self, raw: &str) -> String {
        raw.to_string()
    }
}