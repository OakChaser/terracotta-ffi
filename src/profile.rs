use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
#[repr(u8)]
pub enum ProfileKind {
    Host = 0,
    Local = 1,
    Guest = 2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Profile {
    machine_id: String,
    name: String,
    vendor: String,
    kind: ProfileKind,
}

#[derive(Debug, Clone)]
pub struct ProfileSnapshot {
    pub machine_id: String,
    pub name: String,
    pub vendor: String,
    pub kind: ProfileKind,
}

impl ProfileSnapshot {
    pub fn into_profile(self) -> Profile {
        Profile {
            machine_id: self.machine_id,
            name: self.name,
            vendor: self.vendor,
            kind: self.kind,
        }
    }
}

impl Profile {
    pub fn get_machine_id(&self) -> &str {
        &self.machine_id
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_vendor(&self) -> &str {
        &self.vendor
    }

    pub fn get_kind(&self) -> ProfileKind {
        self.kind
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}
