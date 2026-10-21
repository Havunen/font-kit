// font-kit/src/sources/core_text.rs
//
// Copyright © 2018 The Pathfinder Project Developers.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A source that contains the installed fonts on macOS.

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_text::font::new_from_descriptor;
use core_text::font_collection::{self, CTFontCollection};
use core_text::font_descriptor::{self, CTFontDescriptor};
use core_text::font_manager;
use std::any::Any;
use std::collections::HashMap;
use std::f32;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::SelectionError;
use crate::family_handle::FamilyHandle;
use crate::family_name::FamilyName;
use crate::file_type::FileType;
use crate::font::Font;
use crate::handle::Handle;
use crate::loaders::core_text::{self as core_text_loader, FONT_WEIGHT_MAPPING};
use crate::properties::{Properties, Stretch, Weight};
use crate::source::Source;
use crate::utils;

/// A source that contains the installed fonts on macOS.
#[allow(missing_debug_implementations)]
#[allow(missing_copy_implementations)]
pub struct CoreTextSource;

impl CoreTextSource {
    /// Opens a new connection to the system font source.
    ///
    /// (Note that this doesn't actually do any Mach communication to the font server; that is done
    /// lazily on demand by the Core Text/Core Graphics API.)
    #[inline]
    pub fn new() -> CoreTextSource {
        CoreTextSource
    }

    /// Returns paths of all fonts installed on the system.
    pub fn all_fonts(&self) -> Result<Vec<Handle>, SelectionError> {
        let collection = font_collection::create_for_all_families();
        create_handles_from_core_text_collection(collection)
    }

    /// Returns the names of all families installed on the system.
    pub fn all_families(&self) -> Result<Vec<String>, SelectionError> {
        let core_text_family_names = font_manager::copy_available_font_family_names();
        let mut families = Vec::with_capacity(core_text_family_names.len() as usize);
        for core_text_family_name in core_text_family_names.iter() {
            families.push(core_text_family_name.to_string())
        }
        Ok(families)
    }

    /// Looks up a font family by name and returns the handles of all the fonts in that family.
    pub fn select_family_by_name(&self, family_name: &str) -> Result<FamilyHandle, SelectionError> {
        let attributes: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[(
            CFString::new("NSFontFamilyAttribute"),
            CFString::new(family_name).as_CFType(),
        )]);

        let descriptor = font_descriptor::new_from_attributes(&attributes);
        let descriptors = CFArray::from_CFTypes(&[descriptor]);
        let collection = font_collection::new_from_descriptors(&descriptors);
        let handles = create_handles_from_core_text_collection(collection)?;
        Ok(FamilyHandle::from_font_handles(handles.into_iter()))
    }

    /// Selects a font by PostScript name, which should be a unique identifier.
    pub fn select_by_postscript_name(
        &self,
        postscript_name: &str,
    ) -> Result<Handle, SelectionError> {
        let attributes: CFDictionary<CFString, CFType> = CFDictionary::from_CFType_pairs(&[(
            CFString::new("NSFontNameAttribute"),
            CFString::new(postscript_name).as_CFType(),
        )]);

        let descriptor = font_descriptor::new_from_attributes(&attributes);
        let descriptors = CFArray::from_CFTypes(&[descriptor]);
        let collection = font_collection::new_from_descriptors(&descriptors);
        match collection.get_descriptors() {
            None => Err(SelectionError::NotFound),
            Some(descriptors) => create_handle_from_descriptor(&descriptors.get(0).unwrap()),
        }
    }

    /// Performs font matching according to the CSS Fonts Level 3 specification and returns the
    /// handle.
    #[inline]
    pub fn select_best_match(
        &self,
        family_names: &[FamilyName],
        properties: &Properties,
    ) -> Result<Handle, SelectionError> {
        <Self as Source>::select_best_match(self, family_names, properties)
    }
}

impl Default for CoreTextSource {
    #[inline]
    fn default() -> CoreTextSource {
        CoreTextSource::new()
    }
}

impl Source for CoreTextSource {
    fn all_fonts(&self) -> Result<Vec<Handle>, SelectionError> {
        self.all_fonts()
    }

    fn all_families(&self) -> Result<Vec<String>, SelectionError> {
        self.all_families()
    }

    fn select_family_by_name(&self, family_name: &str) -> Result<FamilyHandle, SelectionError> {
        self.select_family_by_name(family_name)
    }

    fn select_by_postscript_name(&self, postscript_name: &str) -> Result<Handle, SelectionError> {
        self.select_by_postscript_name(postscript_name)
    }

    #[inline]
    fn as_any(&self) -> &dyn Any {
        self
    }

    #[inline]
    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }
}

#[allow(dead_code)]
fn css_to_core_text_font_weight(css_weight: Weight) -> f32 {
    core_text_loader::piecewise_linear_lookup(
        f32::max(100.0, css_weight.0) / 100.0 - 1.0,
        &FONT_WEIGHT_MAPPING,
    )
}

#[allow(dead_code)]
fn css_stretchiness_to_core_text_width(css_stretchiness: Stretch) -> f32 {
    let css_stretchiness = utils::clamp(css_stretchiness.0, 0.5, 2.0);
    0.1 * core_text_loader::piecewise_linear_find_index(css_stretchiness, &Stretch::MAPPING) - 0.4
}

fn create_handles_from_core_text_collection(
    collection: CTFontCollection,
) -> Result<Vec<Handle>, SelectionError> {
    let Some(descriptors) = collection.get_descriptors() else {
        return Err(SelectionError::NotFound);
    };

    create_handles_from_core_text_descriptors(&descriptors)
}

fn create_handles_from_core_text_descriptors(
    descriptors: &CFArray<CTFontDescriptor>,
) -> Result<Vec<Handle>, SelectionError> {
    let mut fonts = vec![];
    let mut font_file_info_cache = HashMap::new();

    for index in 0..descriptors.len() {
        let descriptor = descriptors.get(index).unwrap();
        fonts.push(
            create_handle_from_descriptor_with_cache(&descriptor, &mut font_file_info_cache)
                .unwrap_or_else(|_| {
                    let native = new_from_descriptor(&descriptor, 16.);
                    let font = unsafe { Font::from_core_text_font_no_path(native.clone()) };
                    Handle::from_native(&font)
                }),
        );
    }

    if fonts.is_empty() {
        Err(SelectionError::NotFound)
    } else {
        Ok(fonts)
    }
}

fn create_handle_from_descriptor(descriptor: &CTFontDescriptor) -> Result<Handle, SelectionError> {
    create_handle_from_descriptor_with_cache(descriptor, &mut HashMap::new())
}

fn create_handle_from_descriptor_with_cache(
    descriptor: &CTFontDescriptor,
    font_file_info_cache: &mut HashMap<PathBuf, FontFileInfo>,
) -> Result<Handle, SelectionError> {
    let Some(font_path) = descriptor.font_path() else {
        return Err(SelectionError::CannotAccessSource { reason: None });
    };
    let font_file_info = if let Some(font_file_info) = font_file_info_cache.get(&font_path) {
        font_file_info
    } else {
        let font_file_info = FontFileInfo::read_from_path(&font_path)?;
        font_file_info_cache.insert(font_path.clone(), font_file_info);
        font_file_info_cache
            .get(&font_path)
            .expect("font file info should be cached after insertion")
    };

    match font_file_info {
        FontFileInfo::Single => Ok(Handle::from_path(font_path, 0)),
        FontFileInfo::Collection(font_indices_by_postscript_name) => {
            font_indices_by_postscript_name
                .get(&descriptor.font_name())
                .copied()
                .map(|font_index| Handle::from_path(font_path, font_index))
                .ok_or(SelectionError::NotFound)
        }
    }
}

enum FontFileInfo {
    Single,
    Collection(HashMap<String, u32>),
}

impl FontFileInfo {
    fn read_from_path(font_path: &Path) -> Result<Self, SelectionError> {
        let mut file = if let Ok(file) = File::open(font_path) {
            file
        } else {
            return Err(SelectionError::CannotAccessSource { reason: None });
        };

        let mut tag = [0; 4];
        if file.read_exact(&mut tag).is_err() {
            return Err(SelectionError::CannotAccessSource { reason: None });
        }
        if tag != *b"ttcf" {
            return Ok(FontFileInfo::Single);
        }
        if file.seek(SeekFrom::Start(0)).is_err() {
            return Err(SelectionError::CannotAccessSource { reason: None });
        }

        let font_data = if let Ok(font_data) = utils::slurp_file(&mut file) {
            Arc::new(font_data)
        } else {
            return Err(SelectionError::CannotAccessSource { reason: None });
        };

        match Font::analyze_bytes(Arc::clone(&font_data)) {
            Ok(FileType::Collection(font_count)) => {
                let mut font_indices_by_postscript_name = HashMap::new();
                for font_index in 0..font_count {
                    if let Ok(font) = Font::from_bytes(Arc::clone(&font_data), font_index)
                        && let Some(font_postscript_name) = font.postscript_name()
                    {
                        font_indices_by_postscript_name
                            .entry(font_postscript_name)
                            .or_insert(font_index);
                    }
                }

                Ok(FontFileInfo::Collection(font_indices_by_postscript_name))
            }
            Ok(FileType::Single) => Ok(FontFileInfo::Single),
            Err(e) => Err(SelectionError::CannotAccessSource {
                reason: Some(format!("{:?} error on path {:?}", e, font_path).into()),
            }),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::properties::{Stretch, Weight};

    #[cfg(target_os = "macos")]
    static TEST_FONT_FILE_PATH: &str = "resources/tests/eb-garamond/EBGaramond12-Regular.otf";
    #[cfg(target_os = "macos")]
    static TEST_FONT_COLLECTION_FILE_PATH: &str = "resources/tests/eb-garamond/EBGaramond12.otc";
    #[cfg(target_os = "macos")]
    static TEST_FONT_COLLECTION_POSTSCRIPT_NAMES: [&str; 2] =
        ["EBGaramond12-Italic", "EBGaramond12-Regular"];

    #[cfg(target_os = "macos")]
    fn test_descriptors_from_file(
        path: &str,
    ) -> core_foundation::array::CFArray<core_text::font_descriptor::CTFontDescriptor> {
        use core_foundation::base::TCFType;
        use core_foundation::url::CFURL;

        let url = CFURL::from_path(path, false).expect("test font should have a valid file URL");
        let descriptors_ref = unsafe {
            core_text::font_manager::CTFontManagerCreateFontDescriptorsFromURL(
                url.as_concrete_TypeRef(),
            )
        };
        assert!(
            !descriptors_ref.is_null(),
            "Core Text should expose descriptors for the test font"
        );
        unsafe { TCFType::wrap_under_create_rule(descriptors_ref) }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn create_handles_from_core_text_collection_keeps_pathless_descriptors() {
        use core_foundation::array::CFArray;
        use core_text::font_manager::create_font_descriptor;

        let font_data = std::fs::read(TEST_FONT_FILE_PATH).unwrap();
        let descriptor = create_font_descriptor(&font_data).unwrap();
        assert!(
            descriptor.font_path().is_none(),
            "descriptor created from memory should not depend on a filesystem path"
        );
        let descriptors = CFArray::from_CFTypes(&[descriptor]);

        let handles = super::create_handles_from_core_text_descriptors(&descriptors)
            .expect("pathless Core Text descriptors should remain selectable");
        assert_eq!(handles.len(), 1);

        let font = handles[0]
            .load()
            .expect("pathless Core Text descriptor handle should load");
        assert_eq!(
            font.postscript_name().as_deref(),
            Some("EBGaramond12-Regular")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn create_handles_from_core_text_collection_does_not_eagerly_copy_file_fonts() {
        let descriptors = test_descriptors_from_file(TEST_FONT_COLLECTION_FILE_PATH);
        let handles = super::create_handles_from_core_text_descriptors(&descriptors)
            .expect("test Core Text descriptors should produce handles");

        assert_eq!(handles.len(), 2);
        assert!(
            handles
                .iter()
                .all(|handle| matches!(handle, crate::handle::Handle::Path { .. })),
            "Core Text source enumeration should use reloadable path handles for file-backed fonts"
        );

        let mut postscript_names = handles
            .iter()
            .map(|handle| {
                handle
                    .load()
                    .unwrap()
                    .postscript_name()
                    .expect("test font should have a PostScript name")
            })
            .collect::<Vec<_>>();
        postscript_names.sort();

        assert_eq!(postscript_names, TEST_FONT_COLLECTION_POSTSCRIPT_NAMES);
    }

    #[test]
    fn test_css_to_core_text_font_weight() {
        // Exact matches
        assert_eq!(super::css_to_core_text_font_weight(Weight(100.0)), -0.7);
        assert_eq!(super::css_to_core_text_font_weight(Weight(400.0)), 0.0);
        assert_eq!(super::css_to_core_text_font_weight(Weight(700.0)), 0.4);
        assert_eq!(super::css_to_core_text_font_weight(Weight(900.0)), 0.8);

        // Linear interpolation
        assert_eq!(super::css_to_core_text_font_weight(Weight(450.0)), 0.1);
    }

    #[test]
    fn test_css_to_core_text_font_stretch() {
        // Exact matches
        assert_eq!(
            super::css_stretchiness_to_core_text_width(Stretch(1.0)),
            0.0
        );
        assert_eq!(
            super::css_stretchiness_to_core_text_width(Stretch(0.5)),
            -0.4
        );
        assert_eq!(
            super::css_stretchiness_to_core_text_width(Stretch(2.0)),
            0.4
        );

        // Linear interpolation
        assert_eq!(
            super::css_stretchiness_to_core_text_width(Stretch(1.7)),
            0.34
        );
    }
}
