//! Closed broker-side sources for business recovery authority.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeMap;
use std::sync::Arc;

use super::dedicated_transport::{
    requery_n02_dedicated_authority, requery_p01_dedicated_authority, DedicatedConformanceRoute,
};
use super::generic_transport::GenericTerminalAuthorityAdapter;
use super::terminal_authority::{
    AuthorityDescriptor, AuthorityQuery, AuthorityQueryFailure, TerminalAuthorityPort,
    TerminalTemplateBinding,
};
use super::IntentSnapshot;
use crate::durable_delivery::DurableDeliveryCoordinator;
use crate::event::dispatcher::AuditAuthorityResourceBinding;
use crate::event::{AuditDispatcher, NewsFlashWindow};
use crate::monitor::push_job::{AuthorityClass, CanonicalValue, ChannelId, DecisionId};

pub(super) type CoordinatorStorageBinding = (String, u64, u64, String, String);

pub(super) enum ActivationBusinessSource {
    Generic {
        coordinator: Arc<DurableDeliveryCoordinator>,
        binding: CoordinatorStorageBinding,
        descriptor: AuthorityDescriptor,
    },
    P01 {
        coordinator: Arc<DurableDeliveryCoordinator>,
        binding: CoordinatorStorageBinding,
        route: DedicatedConformanceRoute,
        descriptor: AuthorityDescriptor,
    },
    N02 {
        audit: Arc<AuditDispatcher>,
        binding: AuditAuthorityResourceBinding,
        route: DedicatedConformanceRoute,
        window: NewsFlashWindow,
        descriptor: AuthorityDescriptor,
    },
}

impl ActivationBusinessSource {
    pub(super) fn generic(coordinator: Arc<DurableDeliveryCoordinator>) -> Result<Self, ()> {
        let binding = coordinator.activation_storage_binding().map_err(|_| ())?;
        let descriptor = GenericTerminalAuthorityAdapter::try_new(&coordinator)
            .map_err(|_| ())?
            .descriptor()
            .clone();
        Ok(Self::Generic {
            coordinator,
            binding,
            descriptor,
        })
    }

    pub(super) fn p01(
        coordinator: Arc<DurableDeliveryCoordinator>,
        template: TerminalTemplateBinding,
        required_channel: ChannelId,
    ) -> Result<Self, ()> {
        let binding = coordinator.activation_storage_binding().map_err(|_| ())?;
        let route =
            DedicatedConformanceRoute::try_new(template, required_channel).map_err(|_| ())?;
        if route.authority_class() != AuthorityClass::P01Dedicated {
            return Err(());
        }
        let descriptor = route.authority_descriptor().map_err(|_| ())?;
        Ok(Self::P01 {
            coordinator,
            binding,
            route,
            descriptor,
        })
    }

    pub(super) fn n02(
        audit: Arc<AuditDispatcher>,
        template: TerminalTemplateBinding,
        required_channel: ChannelId,
        window: NewsFlashWindow,
        year: i32,
    ) -> Result<Self, ()> {
        let binding = audit.activation_authority_binding(year).map_err(|_| ())?;
        let route =
            DedicatedConformanceRoute::try_new(template, required_channel).map_err(|_| ())?;
        if route.authority_class() != AuthorityClass::N02Dedicated {
            return Err(());
        }
        let descriptor = route.authority_descriptor().map_err(|_| ())?;
        Ok(Self::N02 {
            audit,
            binding,
            route,
            window,
            descriptor,
        })
    }

    pub(super) fn class(&self) -> AuthorityClass {
        self.descriptor().authority_class
    }

    pub(super) fn descriptor(&self) -> &AuthorityDescriptor {
        match self {
            Self::Generic { descriptor, .. }
            | Self::P01 { descriptor, .. }
            | Self::N02 { descriptor, .. } => descriptor,
        }
    }

    pub(super) fn coordinator_binding(&self) -> Option<&CoordinatorStorageBinding> {
        match self {
            Self::Generic { binding, .. } | Self::P01 { binding, .. } => Some(binding),
            Self::N02 { .. } => None,
        }
    }

    pub(super) fn namespace(&self) -> &str {
        match self {
            Self::Generic { binding, .. } | Self::P01 { binding, .. } => &binding.3,
            Self::N02 { binding, .. } => &binding.namespace,
        }
    }

    #[cfg(test)]
    pub(super) fn n02_window(&self) -> Option<NewsFlashWindow> {
        match self {
            Self::N02 { window, .. } => Some(*window),
            Self::Generic { .. } | Self::P01 { .. } => None,
        }
    }

    #[cfg(test)]
    pub(super) fn coordinator_binding_mut(&mut self) -> &mut CoordinatorStorageBinding {
        match self {
            Self::Generic { binding, .. } | Self::P01 { binding, .. } => binding,
            Self::N02 { .. } => panic!("N02 has no coordinator binding"),
        }
    }

    #[cfg(test)]
    pub(super) fn mutate_dedicated_canonical_field(&mut self, index: usize) {
        match self {
            Self::P01 {
                binding,
                route,
                descriptor,
                ..
            } => match index {
                0 => binding.0.push_str(".other"),
                1 => binding.1 += 1,
                2 => binding.2 += 1,
                3 => binding.3.push_str("-other"),
                4 => binding.4.push_str("-other"),
                5 => descriptor.authority_class = AuthorityClass::N02Dedicated,
                6 => {
                    descriptor.durable_schema_version =
                        crate::monitor::push_job::DurableSchemaVersion::try_new(
                            "p01-durable-other".to_owned(),
                        )
                        .unwrap()
                }
                7 => {
                    *route = DedicatedConformanceRoute::try_new(
                        route.template().clone(),
                        ChannelId::try_new("TEST_CODE_OTHER_CHANNEL".to_owned()).unwrap(),
                    )
                    .unwrap()
                }
                _ => panic!("P01 dedicated canonical field index"),
            },
            Self::N02 {
                binding,
                route,
                window,
                descriptor,
                ..
            } => match index {
                0 => binding.root_path.push_str(".other"),
                1 => binding.namespace.push_str("-other"),
                2 => binding.root_device += 1,
                3 => binding.root_inode += 1,
                4 => binding.root_mode ^= 1,
                5 => binding.root_uid += 1,
                6 => binding.root_links += 1,
                7 => binding.year += 1,
                8 => binding.lock_device += 1,
                9 => binding.lock_inode += 1,
                10 => binding.lock_mode ^= 1,
                11 => binding.lock_uid += 1,
                12 => binding.lock_links += 1,
                13 => binding.jsonl_device += 1,
                14 => binding.jsonl_inode += 1,
                15 => binding.jsonl_mode ^= 1,
                16 => binding.jsonl_uid += 1,
                17 => binding.jsonl_links += 1,
                18 => descriptor.authority_class = AuthorityClass::P01Dedicated,
                19 => {
                    descriptor.durable_schema_version =
                        crate::monitor::push_job::DurableSchemaVersion::try_new(
                            "news-flash-authority-other".to_owned(),
                        )
                        .unwrap()
                }
                20 => {
                    *route = DedicatedConformanceRoute::try_new(
                        route.template().clone(),
                        ChannelId::try_new("TEST_CODE_OTHER_CHANNEL".to_owned()).unwrap(),
                    )
                    .unwrap()
                }
                21 => *window = NewsFlashWindow::H1130,
                _ => panic!("N02 dedicated canonical field index"),
            },
            Self::Generic { .. } => panic!("Generic has no dedicated canonical fields"),
        }
    }

    pub(super) fn check_resources(&self) -> Result<(), ()> {
        match self {
            Self::Generic {
                coordinator,
                binding,
                ..
            }
            | Self::P01 {
                coordinator,
                binding,
                ..
            } => {
                if &coordinator.activation_storage_binding().map_err(|_| ())? != binding {
                    return Err(());
                }
            }
            Self::N02 { audit, binding, .. } => {
                if &audit
                    .activation_authority_binding(binding.year)
                    .map_err(|_| ())?
                    != binding
                {
                    return Err(());
                }
            }
        }
        Ok(())
    }

    pub(super) fn requery(
        &self,
        snapshot: &IntentSnapshot,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        let query = match self {
            Self::Generic { coordinator, .. } => {
                GenericTerminalAuthorityAdapter::try_new(coordinator)
                    .map_err(|_| AuthorityQueryFailure)?
                    .requery_terminal(decision_id)
            }
            Self::P01 {
                coordinator, route, ..
            } => requery_p01_dedicated_authority(snapshot, route, coordinator),
            Self::N02 {
                audit,
                binding,
                route,
                window,
                ..
            } => requery_n02_dedicated_authority(snapshot, *window, route, audit, binding),
        }?;
        self.check_resources().map_err(|_| AuthorityQueryFailure)?;
        Ok(query)
    }

    pub(super) fn dedicated_canonical_fields(
        &self,
    ) -> Result<BTreeMap<&'static str, CanonicalValue>, ()> {
        match self {
            Self::P01 {
                binding,
                route,
                descriptor,
                ..
            } => {
                let source_binding = BTreeMap::from([
                    (
                        "kind",
                        CanonicalValue::String("P01DurableDeliveryCoordinator".to_owned()),
                    ),
                    ("path", CanonicalValue::String(binding.0.clone())),
                    ("device", CanonicalValue::Unsigned(binding.1)),
                    ("inode", CanonicalValue::Unsigned(binding.2)),
                    ("namespace", CanonicalValue::String(binding.3.clone())),
                    ("owner_instance", CanonicalValue::String(binding.4.clone())),
                ]);
                Ok(BTreeMap::from([
                    (
                        "authority_class",
                        CanonicalValue::String(authority_class_label(descriptor.authority_class)),
                    ),
                    (
                        "authority_schema_version",
                        CanonicalValue::String(
                            descriptor.durable_schema_version.as_str().to_owned(),
                        ),
                    ),
                    (
                        "required_channel",
                        CanonicalValue::String(route.required_channel().as_str().to_owned()),
                    ),
                    ("source_binding", CanonicalValue::Object(source_binding)),
                ]))
            }
            Self::N02 {
                binding,
                route,
                window,
                descriptor,
                ..
            } => {
                let source_binding = BTreeMap::from([
                    (
                        "kind",
                        CanonicalValue::String("N02AuditDispatcher".to_owned()),
                    ),
                    ("path", CanonicalValue::String(binding.root_path.clone())),
                    (
                        "namespace",
                        CanonicalValue::String(binding.namespace.clone()),
                    ),
                    ("root_device", CanonicalValue::Unsigned(binding.root_device)),
                    ("root_inode", CanonicalValue::Unsigned(binding.root_inode)),
                    (
                        "root_mode",
                        CanonicalValue::Unsigned(u64::from(binding.root_mode)),
                    ),
                    (
                        "root_uid",
                        CanonicalValue::Unsigned(u64::from(binding.root_uid)),
                    ),
                    ("root_links", CanonicalValue::Unsigned(binding.root_links)),
                    ("year", CanonicalValue::String(binding.year.to_string())),
                    ("lock_device", CanonicalValue::Unsigned(binding.lock_device)),
                    ("lock_inode", CanonicalValue::Unsigned(binding.lock_inode)),
                    (
                        "lock_mode",
                        CanonicalValue::Unsigned(u64::from(binding.lock_mode)),
                    ),
                    (
                        "lock_uid",
                        CanonicalValue::Unsigned(u64::from(binding.lock_uid)),
                    ),
                    ("lock_links", CanonicalValue::Unsigned(binding.lock_links)),
                    (
                        "jsonl_device",
                        CanonicalValue::Unsigned(binding.jsonl_device),
                    ),
                    ("jsonl_inode", CanonicalValue::Unsigned(binding.jsonl_inode)),
                    (
                        "jsonl_mode",
                        CanonicalValue::Unsigned(u64::from(binding.jsonl_mode)),
                    ),
                    (
                        "jsonl_uid",
                        CanonicalValue::Unsigned(u64::from(binding.jsonl_uid)),
                    ),
                    ("jsonl_links", CanonicalValue::Unsigned(binding.jsonl_links)),
                ]);
                let mut fields = BTreeMap::from([
                    (
                        "authority_class",
                        CanonicalValue::String(authority_class_label(descriptor.authority_class)),
                    ),
                    (
                        "authority_schema_version",
                        CanonicalValue::String(
                            descriptor.durable_schema_version.as_str().to_owned(),
                        ),
                    ),
                    (
                        "required_channel",
                        CanonicalValue::String(route.required_channel().as_str().to_owned()),
                    ),
                    ("source_binding", CanonicalValue::Object(source_binding)),
                ]);
                fields.insert("window", CanonicalValue::String(window.label().to_owned()));
                Ok(fields)
            }
            Self::Generic { .. } => Err(()),
        }
    }
}

fn authority_class_label(class: AuthorityClass) -> String {
    match class {
        AuthorityClass::GenericCounted => "GenericCounted",
        AuthorityClass::P01Dedicated => "P01Dedicated",
        AuthorityClass::N02Dedicated => "N02Dedicated",
    }
    .to_owned()
}
