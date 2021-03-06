use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use multimap::MultiMap;
use rocket::request::{FromRequest, Outcome};
use rocket::Request;

use log::{debug, warn};

mod time_parser {
    use serde::ser::Error;
    use serde::Serializer;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn serialize<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        return match timestamp.duration_since(UNIX_EPOCH) {
            Ok(t) => serializer.serialize_u64(t.as_millis() as u64),
            Err(_) => Err(S::Error::custom("Error parsing time")),
        };
    }
}

pub struct RemoteAddress(IpAddr);

impl RemoteAddress {
    pub fn ip(&self) -> IpAddr {
        self.0
    }
}

impl<'a, 'r> FromRequest<'a, 'r> for RemoteAddress {
    type Error = ();

    fn from_request(request: &'a Request<'r>) -> Outcome<Self, Self::Error> {
        let ip = request
            .headers()
            .get_one("X-Forwarded-For")
            .and_then(|ip| {
                ip.parse()
                    .map_err(|_| println!("'X-Real-IP' header is malformed: {}", ip))
                    .ok()
            })
            .or_else(|| request.client_ip());
        return if let Some(addr) = ip {
            Outcome::Success(RemoteAddress { 0: addr })
        } else {
            Outcome::Forward(())
        };
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BotInstance {
    pub domain: String,
    pub port: u16
}

#[derive(Serialize, Clone, Debug)]
pub struct AddressEntry {
    pub domain: String,
    pub port: u16,
    #[serde(with = "time_parser")]
    updated: SystemTime
}

impl AddressEntry {
    fn new_from_instance(instance: &BotInstance) -> AddressEntry {
        AddressEntry {
            domain: instance.domain.clone(),
            port: instance.port,
            updated: SystemTime::now(),
        }
    }

    pub fn is_entry_valid(&self, ttl: Duration) -> bool {
        if let Ok(t) = self.updated.elapsed() {
            return t < ttl;
        }
        return false;
    }
}

impl PartialEq for AddressEntry {
    fn eq(&self, other: &Self) -> bool {
        self.domain.eq(&other.domain) && self.port.eq(&other.port)
    }
}

impl PartialEq<BotInstance> for AddressEntry {
    fn eq(&self, other: &BotInstance) -> bool {
        self.domain.eq(&other.domain) && self.port.eq(&other.port)
    }
}

#[derive(Debug)]
pub struct Registry {
    multimap: MultiMap<IpAddr, AddressEntry>,
    ttl: Duration,
}

impl Registry {
    pub fn create(capacity: usize, ttl: Duration) -> Registry {
        debug!(
            "Creating new registry instance with capacity {} and ttl of {} seconds",
            capacity,
            ttl.as_secs()
        );
        Registry {
            multimap: MultiMap::with_capacity(capacity),
            ttl,
        }
    }

    fn insert_unchecked(&mut self, key: IpAddr, value: BotInstance) -> () {
        if let Some(vec) = self.multimap.get_vec_mut(&key) {
            match { vec.iter().position(|e| e == &value) } {
                Some(pos) => {
                    if let Some(entry) = vec.get_mut(pos) {
                        debug!("Updating entry {:?} for key {}", &value, &key);
                        entry.updated = SystemTime::now();
                    }
                }
                None => {
                    debug!("Adding new entry {:?} to key {}", &value, &key);
                    vec.push(AddressEntry::new_from_instance(&value))
                }
            }
        } else {
            debug!("Adding new key {} with entry {:?}", &key, &value);
            self.multimap
                .insert(key, AddressEntry::new_from_instance(&value));
        }
    }

    pub fn insert(&mut self, key: IpAddr, value: BotInstance) -> bool {
        if self.multimap.len() < self.multimap.capacity() {
            self.insert_unchecked(key, value);
            return true;
        } else {
            self.clean();
            if self.multimap.len() < self.multimap.capacity() {
                self.insert_unchecked(key, value);
                return true;
            }
            warn!("Registry is full. Not able to add new entry!")
        }
        return false;
    }

    pub fn get(&self, key: &IpAddr) -> Option<(Vec<AddressEntry>, bool)> {
        let ttl = self.ttl;
        let result = self.multimap.get_vec(&key);
        return match result {
            Some(vec) => {
                let size = vec.len();
                let cleaned: Vec<AddressEntry> = vec
                    .iter()
                    .filter(|item| item.is_entry_valid(ttl))
                    .cloned()
                    .collect();
                let cleaned_size = cleaned.len();
                debug!(
                    "Request from {} matched {:?}. Result is dirty: {}",
                    key,
                    cleaned,
                    size != cleaned_size
                );
                Some((cleaned, size != cleaned_size))
            }
            _ => {
                debug!("Request from {} had no match", key);
                None
            }
        };
    }

    pub fn clean_key(&mut self, key: &IpAddr) {
        debug!("Cleaning key {}", key);
        let ttl = self.ttl;
        let opt_vec = self.multimap.get_vec_mut(&key);
        if let Some(vec) = opt_vec {
            let mut i = 0;
            while i != vec.len() {
                if !(&mut vec[i]).is_entry_valid(ttl) {
                    debug!("Removing entry {:?} from key {}", vec[i], key);
                    vec.remove(i);
                } else {
                    i += 1;
                }
            }
            if vec.is_empty() {
                debug!("Key is empty. Removing key {}", key);
                self.multimap.remove(key);
            }
        }
    }

    pub fn clean(&mut self) -> () {
        debug!("Cleaning Registry");
        let ttl = self.ttl;
        self.multimap.retain(|_, v| v.is_entry_valid(ttl))
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use std::thread;

    #[test]
    fn insert_and_get() -> () {
        let mut reg = Registry::create(5, Duration::from_secs(300));
        let bot_instance = BotInstance {
            domain: "instance.kiu.party".to_string(),
            port: 41234
        };
        reg.insert(IpAddr::V4(Ipv4Addr::LOCALHOST), bot_instance);
        assert_eq!(1, reg.multimap.len());
        let (vec, dirty) = reg.get(&IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap();
        assert_eq!(1, vec.len());
        assert!(!dirty);
        assert_eq!("instance.kiu.party".to_string(), vec[0].domain)
    }

    #[test]
    fn test_ttl() -> () {
        let mut reg = Registry::create(5, Duration::from_secs(4));
        let bot_instance_1 = BotInstance {
            domain: "instance.kiu.party".to_string(),
            port: 41234
        };
        reg.insert(IpAddr::V4(Ipv4Addr::LOCALHOST), bot_instance_1);
        thread::sleep(Duration::from_secs(2));
        let bot_instance_2 = BotInstance {
            domain: "instance.kiu.party".to_string(),
            port: 41235
        };
        reg.insert(IpAddr::V4(Ipv4Addr::LOCALHOST), bot_instance_2);
        thread::sleep(Duration::from_secs(2));
        let (vec, dirty) = reg.get(&IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap();
        assert_eq!(1, vec.len());
        assert!(dirty);
        reg.clean_key(&IpAddr::V4(Ipv4Addr::LOCALHOST));
        let (vec, dirty) = reg.get(&IpAddr::V4(Ipv4Addr::LOCALHOST)).unwrap();
        assert_eq!(1, vec.len());
        assert!(!dirty);
        thread::sleep(Duration::from_secs(2));
        reg.clean_key(&IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(None, reg.get(&IpAddr::V4(Ipv4Addr::LOCALHOST)))
    }
}
