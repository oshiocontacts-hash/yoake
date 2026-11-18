use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::{Item, Type, UseTree, Visibility};

use crate::modules::{self, ScannedFile};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelfTyStatus {
    Local {
        is_struct_enum_union: bool,
        crate_level_visible: bool,
    },
    ForeignOrPrimitive,
    NonPath,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeDeclKind {
    StructEnumUnion,
    TypeAlias,
}

struct TypeDecl {
    kind: TypeDeclKind,
    vis_crate_visible: bool,
    alias_target: Option<Type>,
}

struct UseBinding {
    local_name: Option<String>,
    leading_colon: bool,
    segments: Vec<String>,
    vis_crate_visible: bool,
}

struct ModuleInfo {
    chain_visible: bool,
    types: BTreeMap<String, Vec<TypeDecl>>,
    traits: BTreeSet<String>,
    uses: Vec<UseBinding>,
    children: BTreeSet<String>,
}

pub struct CrateIndex {
    crate_ident: String,
    modules: BTreeMap<Vec<String>, ModuleInfo>,
}

impl CrateIndex {
    pub fn build(
        crate_root: &Path,
        crate_name: &str,
        crate_ident: &str,
        cached_files: &[ScannedFile],
    ) -> Result<Self, String> {
        let mut index = Self {
            crate_ident: crate_ident.to_string(),
            modules: BTreeMap::new(),
        };

        modules::walk_crate_tree(crate_root, crate_name, cached_files, |mod_path, mod_vis, items| {
            let parent_chain_visible = if mod_path.is_empty() {
                true
            } else {
                let parent_path = &mod_path[..mod_path.len() - 1];
                let parent_visible = index
                    .modules
                    .get(parent_path)
                    .is_some_and(|m| m.chain_visible);
                let vis_ok = mod_vis.is_some_and(|v| is_vis_visible_from_crate(v, parent_path));
                parent_visible && vis_ok
            };

            if let Some(last) = mod_path.last() {
                let parent_path = mod_path[..mod_path.len() - 1].to_vec();
                if let Some(parent) = index.modules.get_mut(&parent_path) {
                    parent.children.insert(last.clone());
                }
            }

            let entry = index.modules.entry(mod_path.to_vec()).or_insert_with(|| ModuleInfo {
                chain_visible: parent_chain_visible,
                types: BTreeMap::new(),
                traits: BTreeSet::new(),
                uses: Vec::new(),
                children: BTreeSet::new(),
            });
            entry.chain_visible = entry.chain_visible && parent_chain_visible;

            for item in items {
                match item {
                    Item::Struct(s) => {
                        entry
                            .types
                            .entry(s.ident.to_string())
                            .or_default()
                            .push(TypeDecl {
                                kind: TypeDeclKind::StructEnumUnion,
                                vis_crate_visible: is_vis_visible_from_crate(&s.vis, mod_path),
                                alias_target: None,
                            });
                    }
                    Item::Enum(e) => {
                        entry
                            .types
                            .entry(e.ident.to_string())
                            .or_default()
                            .push(TypeDecl {
                                kind: TypeDeclKind::StructEnumUnion,
                                vis_crate_visible: is_vis_visible_from_crate(&e.vis, mod_path),
                                alias_target: None,
                            });
                    }
                    Item::Union(u) => {
                        entry
                            .types
                            .entry(u.ident.to_string())
                            .or_default()
                            .push(TypeDecl {
                                kind: TypeDeclKind::StructEnumUnion,
                                vis_crate_visible: is_vis_visible_from_crate(&u.vis, mod_path),
                                alias_target: None,
                            });
                    }
                    Item::Type(t) => {
                        entry
                            .types
                            .entry(t.ident.to_string())
                            .or_default()
                            .push(TypeDecl {
                                kind: TypeDeclKind::TypeAlias,
                                vis_crate_visible: is_vis_visible_from_crate(&t.vis, mod_path),
                                alias_target: Some((*t.ty).clone()),
                            });
                    }
                    Item::Trait(t) => {
                        entry.traits.insert(t.ident.to_string());
                    }
                    Item::Use(u) => {
                        let vis_ok = is_vis_visible_from_crate(&u.vis, mod_path);
                        let mut prefix = Vec::new();
                        collect_use_bindings(
                            &u.tree,
                            u.leading_colon.is_some(),
                            vis_ok,
                            &mut prefix,
                            &mut entry.uses,
                        );
                    }
                    Item::Mod(m) if !modules::has_cfg_test(&m.attrs) => {
                        entry.children.insert(m.ident.to_string());
                    }
                    _ => {}
                }
            }
        })?;

        Ok(index)
    }

    pub fn check_self_ty(&self, from_mod: &[String], self_ty: &Type) -> SelfTyStatus {
        self.check_self_ty_depth(from_mod, self_ty, 0)
    }

    fn check_self_ty_depth(&self, from_mod: &[String], mut self_ty: &Type, depth: usize) -> SelfTyStatus {
        if depth > 8 {
            return SelfTyStatus::ForeignOrPrimitive;
        }
        loop {
            match self_ty {
                Type::Group(g) => self_ty = &g.elem,
                Type::Paren(p) => self_ty = &p.elem,
                _ => break,
            }
        }
        let Type::Path(tp) = self_ty else {
            return SelfTyStatus::NonPath;
        };
        if tp.qself.is_some() {
            return SelfTyStatus::NonPath;
        }
        let segs: Vec<String> = tp.path.segments.iter().map(|s| s.ident.to_string()).collect();
        if segs.is_empty() {
            return SelfTyStatus::NonPath;
        }
        if tp.path.leading_colon.is_none() && segs.len() == 1 && is_primitive_type(&segs[0]) {
            return SelfTyStatus::ForeignOrPrimitive;
        }

        match self.resolve_type_path(from_mod, tp.path.leading_colon.is_some(), &segs, depth) {
            Some(ResolvedLocal {
                decl_mod,
                decls,
                reexported_visible,
            }) => {
                let mod_chain_visible = self
                    .modules
                    .get(&decl_mod)
                    .is_some_and(|m| m.chain_visible);
                let crate_level_visible = reexported_visible
                    || (mod_chain_visible && decls.iter().all(|d| d.vis_crate_visible));

                let is_struct_enum_union = decls.iter().any(|d| match d.kind {
                    TypeDeclKind::StructEnumUnion => true,
                    TypeDeclKind::TypeAlias => d.alias_target.as_ref().is_some_and(|target| {
                        matches!(
                            self.check_self_ty_depth(&decl_mod, target, depth + 1),
                            SelfTyStatus::Local {
                                is_struct_enum_union: true,
                                ..
                            }
                        )
                    }),
                });
                SelfTyStatus::Local {
                    is_struct_enum_union,
                    crate_level_visible,
                }
            }
            None => SelfTyStatus::ForeignOrPrimitive,
        }
    }

    fn resolve_type_path(
        &self,
        from_mod: &[String],
        leading_colon: bool,
        segs: &[String],
        depth: usize,
    ) -> Option<ResolvedLocal<'_>> {
        if depth > 8 || segs.is_empty() {
            return None;
        }
        if leading_colon {
            if segs.first().is_some_and(|s| s == &self.crate_ident) {
                return self.resolve_from_mod(&[], &segs[1..], depth + 1);
            }
            return None;
        }
        if segs[0] == "crate" {
            return self.resolve_from_mod(&[], &segs[1..], depth + 1);
        }
        if segs[0] == "self" {
            return self.resolve_from_mod(from_mod, &segs[1..], depth + 1);
        }
        if segs[0] == "super" {
            let supers = segs.iter().take_while(|s| s.as_str() == "super").count();
            if supers > from_mod.len() {
                return None;
            }
            let base = &from_mod[..from_mod.len() - supers];
            return self.resolve_from_mod(base, &segs[supers..], depth + 1);
        }

        if segs.len() == 1 {
            let name = &segs[0];
            // 1. Impl's own module declarations
            if let Some(info) = self.modules.get(from_mod)
                && let Some(decls) = info.types.get(name)
            {
                return Some(ResolvedLocal {
                    decl_mod: from_mod.to_vec(),
                    decls,
                    reexported_visible: false,
                });
            }
            // 2. Impl's module use imports (explicit, then glob)
            if let Some(info) = self.modules.get(from_mod) {
                for ub in &info.uses {
                    if ub.local_name.as_deref() == Some(name.as_str()) {
                        let resolved = self.resolve_use_binding(from_mod, ub, depth + 1)?;
                        let reexported_visible = info.chain_visible && ub.vis_crate_visible;
                        return Some(ResolvedLocal {
                            reexported_visible: resolved.reexported_visible || reexported_visible,
                            ..resolved
                        });
                    }
                }
            }
            // 3. Ancestor modules of from_mod (declarations, then their explicit uses)
            for end in (0..from_mod.len()).rev() {
                let anc = &from_mod[..end];
                if let Some(info) = self.modules.get(anc)
                    && let Some(decls) = info.types.get(name)
                {
                    return Some(ResolvedLocal {
                        decl_mod: anc.to_vec(),
                        decls,
                        reexported_visible: false,
                    });
                }
            }
            // 4. Glob imports in from_mod
            if let Some(info) = self.modules.get(from_mod) {
                for ub in &info.uses {
                    if ub.local_name.is_none()
                        && let Some(target_mod) = self.resolve_module_path(from_mod, ub.leading_colon, &ub.segments, depth + 1)
                        && let Some(target_info) = self.modules.get(&target_mod)
                        && let Some(decls) = target_info.types.get(name)
                    {
                        let reexported_visible = info.chain_visible && ub.vis_crate_visible;
                        return Some(ResolvedLocal {
                            decl_mod: target_mod,
                            decls,
                            reexported_visible,
                        });
                    }
                }
            }
            return None;
        }

        // Multi-segment path without crate::/self::/super::
        let first = &segs[0];
        // Child module of from_mod
        let mut child_mod = from_mod.to_vec();
        child_mod.push(first.clone());
        if self.modules.contains_key(&child_mod) {
            return self.resolve_from_mod(&child_mod, &segs[1..], depth + 1);
        }
        // Use import of a module in from_mod
        if let Some(info) = self.modules.get(from_mod) {
            for ub in &info.uses {
                if ub.local_name.as_deref() == Some(first.as_str()) {
                    if let Some(target_mod) =
                        self.resolve_module_path(from_mod, ub.leading_colon, &ub.segments, depth + 1)
                    {
                        return self.resolve_from_mod(&target_mod, &segs[1..], depth + 1);
                    }
                    return None;
                }
            }
        }
        // Self-crate alias (extern crate self as <crate_ident>)
        if first == &self.crate_ident {
            return self.resolve_from_mod(&[], &segs[1..], depth + 1);
        }
        // Ancestor child module
        for end in (0..from_mod.len()).rev() {
            let mut candidate = from_mod[..end].to_vec();
            candidate.push(first.clone());
            if self.modules.contains_key(&candidate) {
                return self.resolve_from_mod(&candidate, &segs[1..], depth + 1);
            }
        }
        None
    }

    fn resolve_from_mod(
        &self,
        base_mod: &[String],
        rest: &[String],
        depth: usize,
    ) -> Option<ResolvedLocal<'_>> {
        if depth > 8 || rest.is_empty() {
            return None;
        }
        if rest.len() == 1 {
            let name = &rest[0];
            let info = self.modules.get(base_mod)?;
            if let Some(decls) = info.types.get(name) {
                return Some(ResolvedLocal {
                    decl_mod: base_mod.to_vec(),
                    decls,
                    reexported_visible: false,
                });
            }
            for ub in &info.uses {
                if ub.local_name.as_deref() == Some(name.as_str()) {
                    let resolved = self.resolve_use_binding(base_mod, ub, depth + 1)?;
                    let reexported_visible = info.chain_visible && ub.vis_crate_visible;
                    return Some(ResolvedLocal {
                        reexported_visible: resolved.reexported_visible || reexported_visible,
                        ..resolved
                    });
                }
            }
            return None;
        }
        let mut next_mod = base_mod.to_vec();
        next_mod.push(rest[0].clone());
        if self.modules.contains_key(&next_mod) {
            return self.resolve_from_mod(&next_mod, &rest[1..], depth + 1);
        }
        None
    }

    fn resolve_use_binding(
        &self,
        in_mod: &[String],
        ub: &UseBinding,
        depth: usize,
    ) -> Option<ResolvedLocal<'_>> {
        if depth > 8 || ub.segments.is_empty() {
            return None;
        }
        if ub.leading_colon {
            if ub.segments.first().is_some_and(|s| s == &self.crate_ident) {
                return self.resolve_from_mod(&[], &ub.segments[1..], depth + 1);
            }
            return None;
        }
        let first = &ub.segments[0];
        if first == "crate" {
            return self.resolve_from_mod(&[], &ub.segments[1..], depth + 1);
        }
        if first == "self" {
            return self.resolve_from_mod(in_mod, &ub.segments[1..], depth + 1);
        }
        if first == "super" {
            let supers = ub
                .segments
                .iter()
                .take_while(|s| s.as_str() == "super")
                .count();
            if supers > in_mod.len() {
                return None;
            }
            let base = &in_mod[..in_mod.len() - supers];
            return self.resolve_from_mod(base, &ub.segments[supers..], depth + 1);
        }
        if first == &self.crate_ident {
            return self.resolve_from_mod(&[], &ub.segments[1..], depth + 1);
        }
        // Edition 2018+: `use sub::Item` from crate root (or local module)
        if let Some(root) = self.modules.get(&Vec::new())
            && (root.children.contains(first) || root.types.contains_key(first))
        {
            return self.resolve_from_mod(&[], &ub.segments, depth + 1);
        }
        None
    }

    fn resolve_module_path(
        &self,
        in_mod: &[String],
        leading_colon: bool,
        segs: &[String],
        depth: usize,
    ) -> Option<Vec<String>> {
        if depth > 8 || segs.is_empty() {
            return None;
        }
        if leading_colon {
            if segs.first().is_some_and(|s| s == &self.crate_ident) {
                let cand = segs[1..].to_vec();
                return self.modules.contains_key(&cand).then_some(cand);
            }
            return None;
        }
        let first = &segs[0];
        let cand = if first == "crate" || first == &self.crate_ident {
            segs[1..].to_vec()
        } else if first == "self" {
            let mut m = in_mod.to_vec();
            m.extend_from_slice(&segs[1..]);
            m
        } else if first == "super" {
            let supers = segs.iter().take_while(|s| s.as_str() == "super").count();
            if supers > in_mod.len() {
                return None;
            }
            let mut m = in_mod[..in_mod.len() - supers].to_vec();
            m.extend_from_slice(&segs[supers..]);
            m
        } else {
            segs.to_vec()
        };
        self.modules.contains_key(&cand).then_some(cand)
    }

    pub fn resolve_local_type_canon(&self, from_mod: &[String], ty: &Type) -> Option<String> {
        self.resolve_local_type_canon_depth(from_mod, ty, 0)
    }

    fn resolve_local_type_canon_depth(
        &self,
        from_mod: &[String],
        mut ty: &Type,
        depth: usize,
    ) -> Option<String> {
        if depth > 8 {
            return None;
        }
        loop {
            match ty {
                Type::Group(g) => ty = &g.elem,
                Type::Paren(p) => ty = &p.elem,
                _ => break,
            }
        }
        let Type::Path(tp) = ty else {
            return None;
        };
        if tp.qself.is_some() {
            return None;
        }
        let segs: Vec<String> = tp.path.segments.iter().map(|s| s.ident.to_string()).collect();
        let last = segs.last()?.clone();
        if tp.path.leading_colon.is_none() && segs.len() == 1 && is_primitive_type(&last) {
            return Some(last);
        }
        if let Some(resolved) = self.resolve_type_path(from_mod, tp.path.leading_colon.is_some(), &segs, depth) {
            if let Some(alias) = resolved.decls.iter().find(|d| d.kind == TypeDeclKind::TypeAlias)
                && !resolved.decls.iter().any(|d| d.kind == TypeDeclKind::StructEnumUnion)
                && let Some(target) = &alias.alias_target
                && let Some(canon) = self.resolve_local_type_canon_depth(&resolved.decl_mod, target, depth + 1)
            {
                return Some(canon);
            }
            let item_name = if segs.len() == 1
                && let Some(info) = self.modules.get(from_mod)
                && let Some(ub) = info.uses.iter().find(|u| u.local_name.as_deref() == Some(last.as_str()))
                && let Some(orig) = ub.segments.last()
            {
                orig.clone()
            } else {
                last
            };
            let prefix = crate::key::module_prefix(&self.crate_ident, &resolved.decl_mod);
            return Some(format!("{prefix}::{item_name}"));
        }
        Some(self.resolve_external_or_path(from_mod, tp.path.leading_colon.is_some(), &segs))
    }

    pub fn resolve_trait_canon(&self, from_mod: &[String], trait_path: &syn::Path) -> Option<String> {
        let segs: Vec<String> = trait_path.segments.iter().map(|s| s.ident.to_string()).collect();
        if segs.is_empty() {
            return None;
        }
        if trait_path.leading_colon.is_none() && segs.len() == 1 {
            let name = &segs[0];
            if let Some(info) = self.modules.get(from_mod) {
                if info.traits.contains(name) {
                    return Some(format!(
                        "{}::{name}",
                        crate::key::module_prefix(&self.crate_ident, from_mod)
                    ));
                }
                for ub in &info.uses {
                    if ub.local_name.as_deref() == Some(name.as_str()) {
                        return Some(self.resolve_external_or_path(from_mod, ub.leading_colon, &ub.segments));
                    }
                }
                for ub in &info.uses {
                    if ub.local_name.is_none()
                        && let Some(target_mod) = self.resolve_module_path(from_mod, ub.leading_colon, &ub.segments, 0)
                        && self.modules.get(&target_mod).is_some_and(|ti| ti.traits.contains(name))
                    {
                        return Some(format!(
                            "{}::{name}",
                            crate::key::module_prefix(&self.crate_ident, &target_mod)
                        ));
                    }
                }
            }
            for end in (0..from_mod.len()).rev() {
                let anc = &from_mod[..end];
                if self.modules.get(anc).is_some_and(|info| info.traits.contains(name)) {
                    return Some(format!(
                        "{}::{name}",
                        crate::key::module_prefix(&self.crate_ident, anc)
                    ));
                }
            }
        }
        Some(self.resolve_external_or_path(from_mod, trait_path.leading_colon.is_some(), &segs))
    }

    fn resolve_external_or_path(&self, from_mod: &[String], leading_colon: bool, segs: &[String]) -> String {
        if segs.is_empty() {
            return String::new();
        }
        if leading_colon {
            return segs.join("::");
        }
        let first = &segs[0];
        if first == "crate" {
            if segs.len() == 1 {
                return self.crate_ident.clone();
            }
            return format!("{}::{}", self.crate_ident, segs[1..].join("::"));
        }
        if first == &self.crate_ident {
            return segs.join("::");
        }
        if first == "self" {
            let mut m = from_mod.to_vec();
            m.extend_from_slice(&segs[1..]);
            return crate::key::module_prefix(&self.crate_ident, &m);
        }
        if first == "super" {
            let supers = segs.iter().take_while(|s| s.as_str() == "super").count();
            if supers <= from_mod.len() {
                let mut m = from_mod[..from_mod.len() - supers].to_vec();
                m.extend_from_slice(&segs[supers..]);
                return crate::key::module_prefix(&self.crate_ident, &m);
            }
        }
        if segs.len() > 1 {
            let mut child_mod = from_mod.to_vec();
            child_mod.push(first.clone());
            if self.modules.contains_key(&child_mod) {
                return format!(
                    "{}::{}",
                    crate::key::module_prefix(&self.crate_ident, from_mod),
                    segs.join("::")
                );
            }
            if let Some(info) = self.modules.get(from_mod) {
                for ub in &info.uses {
                    if ub.local_name.as_deref() == Some(first.as_str()) {
                        let base = self.resolve_external_or_path(from_mod, ub.leading_colon, &ub.segments);
                        return format!("{base}::{}", segs[1..].join("::"));
                    }
                }
            }
            let root_mod: &[String] = &[];
            if self.modules.get(root_mod).is_some_and(|root| {
                root.children.contains(first)
                    || root.types.contains_key(first)
                    || root.traits.contains(first)
            }) {
                return format!("{}::{}", self.crate_ident, segs.join("::"));
            }
        }
        segs.join("::")
    }
}

struct ResolvedLocal<'a> {
    decl_mod: Vec<String>,
    decls: &'a [TypeDecl],
    reexported_visible: bool,
}

fn is_primitive_type(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
    )
}

fn is_vis_visible_from_crate(vis: &Visibility, containing_mod_path: &[String]) -> bool {
    match vis {
        Visibility::Public(_) => true,
        Visibility::Inherited => false,
        Visibility::Restricted(vr) => {
            let segs: Vec<String> = vr.path.segments.iter().map(|s| s.ident.to_string()).collect();
            if segs == ["crate"] {
                true
            } else if segs == ["super"] {
                containing_mod_path.len() == 1
            } else if segs == ["self"] {
                containing_mod_path.is_empty()
            } else {
                false
            }
        }
    }
}

fn collect_use_bindings(
    tree: &UseTree,
    leading_colon: bool,
    vis_crate_visible: bool,
    prefix: &mut Vec<String>,
    out: &mut Vec<UseBinding>,
) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            collect_use_bindings(&p.tree, leading_colon, vis_crate_visible, prefix, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let name = n.ident.to_string();
            if name == "self" {
                if let Some(last) = prefix.last().cloned() {
                    out.push(UseBinding {
                        local_name: Some(last),
                        leading_colon,
                        segments: prefix.clone(),
                        vis_crate_visible,
                    });
                }
            } else {
                let mut segs = prefix.clone();
                segs.push(name.clone());
                out.push(UseBinding {
                    local_name: Some(name),
                    leading_colon,
                    segments: segs,
                    vis_crate_visible,
                });
            }
        }
        UseTree::Rename(r) => {
            let name = r.ident.to_string();
            let alias = r.rename.to_string();
            let segs = if name == "self" {
                prefix.clone()
            } else {
                let mut s = prefix.clone();
                s.push(name);
                s
            };
            if !segs.is_empty() {
                out.push(UseBinding {
                    local_name: Some(alias),
                    leading_colon,
                    segments: segs,
                    vis_crate_visible,
                });
            }
        }
        UseTree::Glob(_) => {
            out.push(UseBinding {
                local_name: None,
                leading_colon,
                segments: prefix.clone(),
                vis_crate_visible,
            });
        }
        UseTree::Group(g) => {
            for item in &g.items {
                collect_use_bindings(item, leading_colon, vis_crate_visible, prefix, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vis_check_recognizes_pub_crate_and_root_super() {
        let pub_vis: Visibility = syn::parse_str("pub").unwrap();
        let pub_crate: Visibility = syn::parse_str("pub(crate)").unwrap();
        let pub_in_crate: Visibility = syn::parse_str("pub(in crate)").unwrap();
        let pub_super: Visibility = syn::parse_str("pub(super)").unwrap();
        let pub_in_sub: Visibility = syn::parse_str("pub(in crate::a)").unwrap();
        let priv_vis: Visibility = Visibility::Inherited;

        let depth1 = vec!["a".to_string()];
        let depth2 = vec!["a".to_string(), "b".to_string()];

        assert!(is_vis_visible_from_crate(&pub_vis, &depth2));
        assert!(is_vis_visible_from_crate(&pub_crate, &depth2));
        assert!(is_vis_visible_from_crate(&pub_in_crate, &depth2));
        assert!(is_vis_visible_from_crate(&pub_super, &depth1));
        assert!(!is_vis_visible_from_crate(&pub_super, &depth2));
        assert!(!is_vis_visible_from_crate(&pub_in_sub, &depth2));
        assert!(!is_vis_visible_from_crate(&priv_vis, &depth1));
    }
}
