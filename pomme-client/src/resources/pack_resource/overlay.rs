use std::collections::BTreeSet;
use std::io;
use std::rc::Rc;

use super::{PackResources, ReadStream, ResourceOrigin, SharedPackResources, validate_query};
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// One stack contribution. Content and sidecars independently search the same
/// children, while returned handles pin the exact winning child.
pub(super) struct OverlayPackResources {
    origin: ResourceOrigin,
    children: Vec<SharedPackResources>,
}

impl OverlayPackResources {
    pub(super) fn new(origin: ResourceOrigin, children: Vec<SharedPackResources>) -> Self {
        Self { origin, children }
    }

    fn locate(&self, id: &ResourceId) -> Result<Option<ResourceContent>, ResourceError> {
        for child in self.children.iter().rev() {
            if let Some(content) = child.clone().get_resource(id)? {
                return Ok(Some(content));
            }
        }
        Ok(None)
    }
}

impl PackResources for OverlayPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceContent>, ResourceError> {
        self.locate(id)
    }

    fn open(&self, id: &ResourceId) -> io::Result<ReadStream> {
        self.locate(id)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "overlay resource is absent"))?
            .open()
            .map_err(io::Error::other)
    }

    fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError> {
        let mut namespaces = BTreeSet::new();
        for child in &self.children {
            namespaces.extend(child.namespaces()?);
        }
        Ok(namespaces)
    }

    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        validate_query(Some(namespace), prefix)?;
        let mut resources = BTreeSet::new();
        for child in &self.children {
            resources.extend(child.list_resources(namespace, prefix)?);
        }
        Ok(resources)
    }
}
