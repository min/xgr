use super::graph::{
    mapped_id, phase_name, xcode_reference_acronym, PbxObject, PbxValue,
};
use super::{display_name, PbxGenerator};
use md5::Digest;
use std::collections::{HashMap, HashSet};

pub(super) struct XcodeReferenceGenerator<'a> {
    generator: &'a PbxGenerator<'a>,
    pub(super) state: ReferenceState,
}

#[derive(Default)]
pub(super) struct ReferenceState {
    pub(super) output: HashMap<String, String>,
    references: HashSet<String>,
    product_contexts: HashMap<String, String>,
}

impl ReferenceState {
    fn fix(&mut self, id: &str, isa: &str, identifiers: &[&str]) {
        if self.output.contains_key(id) {
            return;
        }
        let acronym = xcode_reference_acronym(isa);
        let mut counter = 1;
        loop {
            let seed = format!("{acronym}_{isa}-{}_{}", identifiers.join("-"), counter);
            let digest = format!("{:x}", md5::Md5::digest(seed.as_bytes())).to_uppercase();
            let reference = digest[..24].to_owned();
            if !self.references.contains(&reference) {
                self.references.insert(reference.clone());
                self.output.insert(id.to_owned(), reference);
                return;
            }
            counter += 1;
        }
    }
}

impl<'a> XcodeReferenceGenerator<'a> {
    pub(super) fn new(generator: &'a PbxGenerator<'a>) -> Self {
        Self {
            generator,
            state: ReferenceState::default(),
        }
    }

    pub(super) fn generate(&mut self, root_id: &str) {
        let Some(project) = self.object(root_id) else {
            return;
        };
        let project_name = self.generator.project.name.as_str();
        let project_identifiers = vec![project_name.to_owned()];
        let project_isa = project.isa;
        self.collect_product_contexts(project);

        let package_ids = array_refs(project, "packageReferences");
        let target_ids = array_refs(project, "targets");
        let main_group_id = ref_field(project, "mainGroup");
        let products_group_id = ref_field(project, "productRefGroup");
        let project_config_list_id = ref_field(project, "buildConfigurationList");

        self.state
            .fix(root_id, project_isa, &borrowed_slice(&project_identifiers));

        for package_id in &package_ids {
            let Some(package) = self.object(package_id) else {
                continue;
            };
            let identifier = match package.isa {
                "XCRemoteSwiftPackageReference" => string_field(package, "repositoryURL")
                    .map(str::to_owned)
                    .or_else(|| package.comment.clone()),
                "XCLocalSwiftPackageReference" => {
                    string_field(package, "relativePath").map(str::to_owned)
                }
                _ => None,
            };
            if let Some(identifier) = identifier {
                let isa = package.isa;
                self.state
                    .fix(package_id, isa, &[project_name, identifier.as_str()]);
            }
        }

        for target_id in &target_ids {
            let Some(target) = self.object(target_id) else {
                continue;
            };
            let target_isa = target.isa;
            let Some(target_name) = string_field(target, "name").map(str::to_owned) else {
                continue;
            };

            let product_ids = array_refs(target, "packageProductDependencies");
            let dep_ids = array_refs(target, "dependencies");

            for product_id in &product_ids {
                let Some(product) = self.object(product_id) else {
                    continue;
                };
                let product_isa = product.isa;
                let Some(product_name) = string_field(product, "productName").map(str::to_owned)
                else {
                    continue;
                };
                self.state.fix(
                    product_id,
                    product_isa,
                    &[project_name, target_name.as_str(), product_name.as_str()],
                );
            }

            for dependency_id in &dep_ids {
                let Some(dependency) = self.object(dependency_id) else {
                    continue;
                };
                let Some(product_id) = ref_field(dependency, "productRef").map(str::to_owned)
                else {
                    continue;
                };
                let Some(product) = self.object(&product_id) else {
                    continue;
                };
                let product_isa = product.isa;
                let Some(product_name) = string_field(product, "productName").map(str::to_owned)
                else {
                    continue;
                };
                if let Some(plugin_name) = product_name.strip_prefix("plugin:") {
                    self.state.fix(
                        &product_id,
                        product_isa,
                        &[project_name, target_name.as_str(), plugin_name],
                    );
                }
            }

            self.state
                .fix(target_id, target_isa, &[project_name, target_name.as_str()]);
        }

        if let Some(main_group_id) = main_group_id {
            self.generate_group_references(main_group_id, &project_identifiers);
        }
        if let Some(products_group_id) = products_group_id {
            self.generate_group_references(products_group_id, &project_identifiers);
        }

        for target_id in &target_ids {
            let Some(target) = self.object(target_id) else {
                continue;
            };
            let Some(target_name) = string_field(target, "name").map(str::to_owned) else {
                continue;
            };
            let identifiers = vec![project_name.to_owned(), target_name];
            let config_list_id = ref_field(target, "buildConfigurationList").map(str::to_owned);
            let phase_ids = array_refs(target, "buildPhases");
            let target_dep_ids = array_refs(target, "dependencies");
            if let Some(config_list_id) = config_list_id {
                self.generate_configuration_list_references(&config_list_id, &identifiers);
            }
            for phase_id in &phase_ids {
                self.generate_build_phase_references(phase_id, &identifiers);
            }
            for dependency_id in &target_dep_ids {
                self.generate_target_dependency_references(dependency_id, &identifiers);
            }
        }

        if let Some(config_list_id) = project_config_list_id {
            self.generate_configuration_list_references(config_list_id, &project_identifiers);
        }
    }

    fn collect_product_contexts(&mut self, project: &PbxObject) {
        let target_ids = array_refs(project, "targets");
        for target_id in target_ids {
            let Some(target) = self.object(&target_id) else {
                continue;
            };
            let Some(product_id) = ref_field(target, "productReference").map(str::to_owned) else {
                continue;
            };
            if let Some(target_name) = string_field(target, "name").map(str::to_owned) {
                self.state.product_contexts.insert(product_id, target_name);
            }
        }
    }

    fn generate_group_references(&mut self, group_id: &str, identifiers: &[String]) {
        let Some(group) = self.object(group_id) else {
            return;
        };
        let group_isa = group.isa;
        let file_name = self.file_name(group);
        let child_ids = array_refs(group, "children");
        let mut identifiers = identifiers.to_vec();
        if let Some(file_name) = file_name {
            identifiers.push(file_name);
        }
        self.state
            .fix(group_id, group_isa, &borrowed_slice(&identifiers));

        for child_id in child_ids {
            let Some(child) = self.object(&child_id) else {
                continue;
            };
            let child_isa = child.isa;
            match child_isa {
                "PBXGroup" => self.generate_group_references(&child_id, &identifiers),
                "PBXVariantGroup" | "XCVersionGroup" => {
                    self.generate_variant_group_references(&child_id, &identifiers)
                }
                "PBXFileReference" => self.generate_file_reference(&child_id, &identifiers),
                _ => {}
            }
        }
    }

    fn generate_variant_group_references(&mut self, group_id: &str, identifiers: &[String]) {
        let Some(group) = self.object(group_id) else {
            return;
        };
        let group_isa = group.isa;
        let file_name = self.file_name(group);
        let child_ids = array_refs(group, "children");
        let mut identifiers = identifiers.to_vec();
        if let Some(file_name) = file_name {
            identifiers.push(file_name);
        }
        self.state
            .fix(group_id, group_isa, &borrowed_slice(&identifiers));
        for child_id in child_ids {
            self.generate_file_reference(&child_id, &identifiers);
        }
    }

    fn generate_file_reference(&mut self, file_id: &str, identifiers: &[String]) {
        let Some(file_ref) = self.object(file_id) else {
            return;
        };
        let file_isa = file_ref.isa;
        let file_name = self.file_name(file_ref);
        let mut identifiers = identifiers.to_vec();
        if let Some(file_name) = file_name {
            identifiers.push(file_name);
        }
        if let Some(context) = self.state.product_contexts.get(file_id) {
            identifiers.push(context.clone());
        }
        self.state
            .fix(file_id, file_isa, &borrowed_slice(&identifiers));
    }

    fn generate_configuration_list_references(
        &mut self,
        config_list_id: &str,
        identifiers: &[String],
    ) {
        let Some(config_list) = self.object(config_list_id) else {
            return;
        };
        let config_list_isa = config_list.isa;
        let config_ids = array_refs(config_list, "buildConfigurations");
        self.state
            .fix(config_list_id, config_list_isa, &borrowed_slice(identifiers));
        for config_id in config_ids {
            let Some(config) = self.object(&config_id) else {
                continue;
            };
            let config_isa = config.isa;
            let Some(name) = string_field(config, "name").map(str::to_owned) else {
                continue;
            };
            let mut identifiers = identifiers.to_vec();
            identifiers.push(name);
            self.state
                .fix(&config_id, config_isa, &borrowed_slice(&identifiers));
        }
    }

    fn generate_build_phase_references(&mut self, phase_id: &str, identifiers: &[String]) {
        let Some(phase) = self.object(phase_id) else {
            return;
        };
        let phase_isa = phase.isa;
        let name = phase_name(phase);
        let build_file_ids = array_refs(phase, "files");
        let mut identifiers = identifiers.to_vec();
        if let Some(name) = name {
            identifiers.push(name);
        }
        self.state
            .fix(phase_id, phase_isa, &borrowed_slice(&identifiers));

        for build_file_id in build_file_ids {
            let Some(build_file) = self.object(&build_file_id) else {
                continue;
            };
            let build_file_isa = build_file.isa;
            let file_ref_id = ref_field(build_file, "fileRef").map(str::to_owned);
            let mut build_file_identifiers = identifiers.clone();
            if let Some(file_ref_id) = file_ref_id {
                if let Some(file_ref) = self.state.output.get(&file_ref_id) {
                    build_file_identifiers.push(file_ref.clone());
                }
            }
            self.state.fix(
                &build_file_id,
                build_file_isa,
                &borrowed_slice(&build_file_identifiers),
            );
        }
    }

    fn generate_target_dependency_references(
        &mut self,
        dependency_id: &str,
        identifiers: &[String],
    ) {
        let Some(dependency) = self.object(dependency_id) else {
            return;
        };
        let dependency_isa = dependency.isa;
        let proxy_id = ref_field(dependency, "targetProxy").map(str::to_owned);
        let target_id = ref_field(dependency, "target").map(str::to_owned);
        let mut dependency_identifiers = identifiers.to_vec();

        if let Some(proxy_id) = proxy_id.as_deref() {
            let Some(proxy) = self.object(proxy_id) else {
                return;
            };
            let proxy_isa = proxy.isa;
            if let Some(remote_id) = string_field(proxy, "remoteGlobalIDString").map(str::to_owned)
            {
                let mapped_remote_id = mapped_id(&remote_id, &self.state.output).into_owned();
                let mut proxy_identifiers = identifiers.to_vec();
                proxy_identifiers.push(mapped_remote_id);
                self.state
                    .fix(proxy_id, proxy_isa, &borrowed_slice(&proxy_identifiers));
            }
        }
        if let Some(target_id) = target_id {
            dependency_identifiers.push(mapped_id(&target_id, &self.state.output).into_owned());
        }
        if let Some(proxy_id) = proxy_id {
            dependency_identifiers.push(mapped_id(&proxy_id, &self.state.output).into_owned());
        }
        self.state.fix(
            dependency_id,
            dependency_isa,
            &borrowed_slice(&dependency_identifiers),
        );
    }

    fn object(&self, id: &str) -> Option<&'a PbxObject> {
        self.generator.graph.objects.get(id)
    }

    fn file_name(&self, object: &PbxObject) -> Option<String> {
        string_field(object, "name")
            .map(str::to_owned)
            .or_else(|| string_field(object, "path").map(display_name))
            .filter(|value| !value.is_empty())
    }
}

fn ref_field<'a>(object: &'a PbxObject, key: &str) -> Option<&'a str> {
    match object.fields.get(key)? {
        PbxValue::Ref { id, .. } => Some(id.as_str()),
        _ => None,
    }
}

fn array_refs(object: &PbxObject, key: &str) -> Vec<String> {
    match object.fields.get(key) {
        Some(PbxValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                PbxValue::Ref { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn string_field<'a>(object: &'a PbxObject, key: &str) -> Option<&'a str> {
    match object.fields.get(key)? {
        PbxValue::String(value) => Some(value.as_str()),
        _ => None,
    }
}

fn borrowed_slice(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}
