use lazy_static::lazy_static;
use std::collections::HashSet;

// these are routes with data from the auth server that are
// cached in merx to be reserved via the same endpoints

lazy_static! {
    pub static ref CACHED_ENDPOINTS: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.insert("/api/currency_pairs");
        set.insert("/api/exchanges");
        set.insert("/api/exchange_fees");
        set.insert("/api_internal/currency_pairs");
        set.insert("/api_internal/exchanges");
        set.insert("/api_internal/exchange_fees");
        set
    };
}
