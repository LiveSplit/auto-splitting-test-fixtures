//! Measures the IL2CPP struct layout of one release from the `libil2cpp`
//! headers its editor ships. A small C++ program prints `offsetof` for every
//! member asr reads, built with MSVC for x64 and for x86, so the numbers are
//! the ones a Windows player's `GameAssembly.dll` has. The 6000.5.0f1
//! numbers match that player's PDB on both widths.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// The folder of the editor package that holds every runtime header.
pub(crate) const HEADERS: &str = "il2cpp/libil2cpp/";

/// The files that state the metadata version a release reads, as
/// `IL2CPP_ASSERT(s_GlobalMetadataHeader->version == N)`. Older releases
/// check it in `MetadataCache.cpp`, newer ones in `GlobalMetadata.cpp`.
pub(crate) const VERSION_CHECKS: &[&str] = &[
    "il2cpp/libil2cpp/vm/MetadataCache.cpp",
    "il2cpp/libil2cpp/vm/GlobalMetadata.cpp",
];

/// Reads the metadata version out of whichever version check a release
/// ships, or `None` when neither names one.
pub(crate) fn metadata_version(headers: &Path) -> Option<u32> {
    VERSION_CHECKS.iter().find_map(|file| {
        let name = file.strip_prefix(HEADERS)?;
        let text = fs::read_to_string(headers.join(name)).ok()?;
        let after = text.split("s_GlobalMetadataHeader->version == ").nth(1)?;
        after
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .ok()
    })
}

/// The members asr reads, as `(struct, member)`. The probe prints each one.
const MEMBERS: &[(&str, &str)] = &[
    ("Il2CppAssembly", "image"),
    ("Il2CppImage", "nameNoExt"),
    ("Il2CppImage", "typeCount"),
    ("Il2CppImage", "typeStart"),
    ("Il2CppImage", "metadataHandle"),
    ("Il2CppClass", "name"),
    ("Il2CppClass", "namespaze"),
    ("Il2CppClass", "declaringType"),
    ("Il2CppClass", "parent"),
    ("Il2CppClass", "fields"),
    ("Il2CppClass", "static_fields"),
    ("Il2CppClass", "instance_size"),
    ("Il2CppClass", "field_count"),
    ("Il2CppGenericClass", "cached_class"),
    ("Il2CppType", "data"),
    ("FieldInfo", "name"),
    ("FieldInfo", "type"),
    ("FieldInfo", "offset"),
];

/// Where MSVC and the Windows SDK sit. The toolset is pinned to 14.29, the
/// one the fixture players are built with.
pub(crate) struct Toolset {
    msvc: PathBuf,
    sdk_include: PathBuf,
    sdk_lib: PathBuf,
}

impl Toolset {
    const MSVC: &'static str =
        r"C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Tools\MSVC\14.29.30133";
    const SDK: &'static str = r"C:\Program Files (x86)\Windows Kits\10";
    const SDK_VERSION: &'static str = "10.0.26100.0";

    pub(crate) fn find() -> io::Result<Self> {
        let toolset = Self {
            msvc: PathBuf::from(Self::MSVC),
            sdk_include: Path::new(Self::SDK).join("Include").join(Self::SDK_VERSION),
            sdk_lib: Path::new(Self::SDK).join("Lib").join(Self::SDK_VERSION),
        };
        for path in [&toolset.msvc, &toolset.sdk_include] {
            if !path.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is not installed", path.display()),
                ));
            }
        }
        Ok(toolset)
    }

    /// Builds and runs the probe for one width and returns what it printed.
    fn run(&self, width: &str, source: &Path, headers: &Path) -> io::Result<String> {
        let exe = source.with_extension(format!("{width}.exe"));
        let arg = |flag: &str, path: &Path| format!("{flag}{}", path.display());
        let out = Command::new(
            self.msvc
                .join("bin")
                .join("Hostx64")
                .join(width)
                .join("cl.exe"),
        )
        .current_dir(source.parent().unwrap_or(Path::new(".")))
        .arg("/nologo")
        .arg(arg("/I", headers))
        .arg(arg("/I", &self.msvc.join("include")))
        .arg(arg("/I", &self.sdk_include.join("ucrt")))
        .arg(arg("/I", &self.sdk_include.join("shared")))
        .arg(arg("/I", &self.sdk_include.join("um")))
        .arg("/DRUNTIME_IL2CPP=1")
        .arg("/DIL2CPP_TARGET_WINDOWS_DESKTOP=1")
        .arg(arg("/Fo", &source.with_extension(format!("{width}.obj"))))
        .arg(arg("/Fe", &exe))
        .arg(source)
        .arg("/link")
        .arg(arg("/LIBPATH:", &self.msvc.join("lib").join(width)))
        .arg(arg("/LIBPATH:", &self.sdk_lib.join("ucrt").join(width)))
        .arg(arg("/LIBPATH:", &self.sdk_lib.join("um").join(width)))
        .output()?;
        if !out.status.success() {
            return Err(io::Error::other(format!(
                "cl.exe failed for {width}:\n{}",
                String::from_utf8_lossy(&out.stdout)
            )));
        }
        let run = Command::new(&exe).output()?;
        Ok(String::from_utf8_lossy(&run.stdout).into_owned())
    }
}

/// The C++ source of the probe. A member a release does not have is skipped
/// through `__if_exists`, which MSVC has for exactly this.
fn probe_source() -> String {
    let mut source = String::from(
        "#include <stddef.h>\n#include <stdio.h>\n#include <string.h>\n\
         #include \"il2cpp-class-internals.h\"\n\
         #define P(T, M) __if_exists(T::M) { printf(#T \".\" #M \" %zu\\n\", offsetof(T, M)); }\n\
         int main()\n{\n",
    );
    for (structure, member) in MEMBERS {
        source.push_str(&format!("    P({structure}, {member});\n"));
    }
    source.push_str(
        "    printf(\"FieldInfo sizeof %zu\\n\", sizeof(FieldInfo));\n\
         Il2CppType t; memset(&t, 0, sizeof t); t.type = (Il2CppTypeEnum)0xff;\n\
         const unsigned char* b = (const unsigned char*)&t;\n\
         for (size_t i = 0; i < sizeof t; i++) if (b[i]) printf(\"Il2CppType.kind %zu\\n\", i);\n\
         return 0;\n}\n",
    );
    source
}

/// One width's layout, member by member.
pub(crate) type Layout = Vec<(String, u64)>;

/// Both widths of one release, with the metadata version its runtime
/// checks for. Two releases are equal when their layouts are. The metadata
/// version is not part of that: asr never reads it, so a release that only
/// bumps it is no boundary.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Measured {
    pub(crate) metadata: Option<u32>,
    pub(crate) x64: Layout,
    pub(crate) x86: Layout,
}

impl PartialEq for Measured {
    fn eq(&self, other: &Self) -> bool {
        self.x64 == other.x64 && self.x86 == other.x86
    }
}

impl std::fmt::Display for Measured {
    /// Shows the few members that tell layouts apart at a glance.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pick = |layout: &Layout, name: &str| {
            layout
                .iter()
                .find(|(member, _)| member == name)
                .map_or("?".to_string(), |(_, v)| format!("{v:#x}"))
        };
        write!(
            f,
            "metadata {} x64 size {} count {} statics {} / x86 size {} count {} statics {}",
            self.metadata.map_or("?".to_string(), |m| m.to_string()),
            pick(&self.x64, "Il2CppClass.instance_size"),
            pick(&self.x64, "Il2CppClass.field_count"),
            pick(&self.x64, "Il2CppClass.static_fields"),
            pick(&self.x86, "Il2CppClass.instance_size"),
            pick(&self.x86, "Il2CppClass.field_count"),
            pick(&self.x86, "Il2CppClass.static_fields"),
        )
    }
}

/// Reads the probe's lines into member and offset pairs.
fn parse(output: &str) -> Layout {
    output
        .lines()
        .filter_map(|line| {
            let (name, value) = line.rsplit_once(' ')?;
            Some((name.to_string(), value.trim().parse().ok()?))
        })
        .collect()
}

/// Measures both widths from headers in `headers`, writing the probe and its
/// binaries into `work`.
pub(crate) fn measure(toolset: &Toolset, headers: &Path, work: &Path) -> io::Result<Measured> {
    fs::create_dir_all(work)?;
    let source = work.join("layout.cpp");
    fs::write(&source, probe_source())?;
    Ok(Measured {
        metadata: metadata_version(headers),
        x64: parse(&toolset.run("x64", &source, headers)?),
        x86: parse(&toolset.run("x86", &source, headers)?),
    })
}

/// Prints one layout as an entry for asr's `il2cpp/builds.rs`, the way the
/// table already writes them.
pub(crate) fn entry(version: &str, metadata: Option<u32>, width: u32, layout: &Layout) -> String {
    let at = |name: &str| {
        layout
            .iter()
            .find(|(member, _)| member == name)
            .map(|(_, offset)| *offset)
    };
    let hex = |name: &str| at(name).map_or("?".to_string(), |v| format!("{v:#x}"));
    let some = |name: &str| at(name).map_or("None".to_string(), |v| format!("Some({v:#x})"));
    let type_start = match (
        at("Il2CppImage.typeStart"),
        at("Il2CppImage.metadataHandle"),
    ) {
        (Some(inline), _) => format!("TypeStart::Inline({inline:#x})"),
        (None, Some(handle)) => format!("TypeStart::Handle({handle:#x})"),
        (None, None) => "TypeStart::?".to_string(),
    };
    let metadata = metadata.map_or("?".to_string(), |m| m.to_string());
    let bits = if width == 64 { "64" } else { "32" };
    let arch = if width == 64 { "x64" } else { "x86" };
    format!(
        "    // Unity {version}, metadata version {metadata}, {arch}.
    Build {{
        unity: (?, ?, ?, ?),
        profile: Profile {{
            pointer_size: PointerSize::Bit{bits},
            assembly: AssemblyOffsets {{
                image: {},
                name: None,
            }},
            image: ImageOffsets {{
                assembly_name: {},
                type_count: {},
                type_start: {type_start},
            }},
            class: ClassOffsets {{
                name: {},
                namespace: {},
                declaring_type: {},
                parent: {},
                fields: {},
                static_fields: {},
                instance_size: {},
                field_count: {},
            }},
            generic: GenericOffsets {{
                cached_class: {},
            }},
            type_: TypeOffsets {{
                data: {},
                kind: {},
            }},
            field: FieldInfoOffsets {{
                name: {},
                type_: {},
                offset: {},
                size: {},
            }},
        }},
    }},",
        hex("Il2CppAssembly.image"),
        some("Il2CppImage.nameNoExt"),
        hex("Il2CppImage.typeCount"),
        hex("Il2CppClass.name"),
        hex("Il2CppClass.namespaze"),
        some("Il2CppClass.declaringType"),
        hex("Il2CppClass.parent"),
        hex("Il2CppClass.fields"),
        hex("Il2CppClass.static_fields"),
        some("Il2CppClass.instance_size"),
        hex("Il2CppClass.field_count"),
        some("Il2CppGenericClass.cached_class"),
        some("Il2CppType.data"),
        some("Il2CppType.kind"),
        hex("FieldInfo.name"),
        some("FieldInfo.type"),
        hex("FieldInfo.offset"),
        hex("FieldInfo sizeof"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_when_only_metadata_differs() {
        let layout = vec![("Il2CppClass.field_count".to_string(), 0x124)];
        let a = Measured {
            metadata: Some(29),
            x64: layout.clone(),
            x86: layout.clone(),
        };
        let b = Measured {
            metadata: Some(31),
            x64: layout.clone(),
            x86: layout,
        };
        assert_eq!(a, b);
        let c = Measured {
            metadata: Some(31),
            x64: vec![("Il2CppClass.field_count".to_string(), 0x120)],
            x86: b.x86.clone(),
        };
        assert_ne!(b, c);
    }

    #[test]
    fn the_metadata_version_comes_from_the_runtime_check() {
        let dir = std::env::temp_dir().join("unity-search-metadata");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(dir.join("vm")).unwrap();
        fs::write(
            dir.join("vm/MetadataCache.cpp"),
            "    IL2CPP_ASSERT(s_GlobalMetadataHeader->version == 24);
",
        )
        .unwrap();
        assert_eq!(metadata_version(&dir), Some(24));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_probe_prints_every_member_once() {
        let source = probe_source();
        for (structure, member) in MEMBERS {
            assert_eq!(
                source.matches(&format!("P({structure}, {member})")).count(),
                1
            );
        }
        assert!(source.contains("Il2CppType.kind"));
    }

    #[test]
    fn parse_keeps_the_member_and_its_offset() {
        let layout = parse("Il2CppClass.name 16\nFieldInfo sizeof 32\nnoise\n");
        assert_eq!(
            layout,
            vec![
                ("Il2CppClass.name".to_string(), 16),
                ("FieldInfo sizeof".to_string(), 32)
            ]
        );
    }

    #[test]
    fn an_entry_takes_the_handle_when_there_is_no_inline_type_start() {
        let layout = vec![
            ("Il2CppImage.metadataHandle".to_string(), 0x28),
            ("Il2CppClass.field_count".to_string(), 0x124),
        ];
        let text = entry("6000.5.0f1", Some(106), 64, &layout);
        assert!(text.contains("type_start: TypeStart::Handle(0x28)"));
        assert!(text.contains("field_count: 0x124"));
        assert!(text.contains("pointer_size: PointerSize::Bit64"));
        assert!(text.contains("metadata version 106, x64"));
    }

    #[test]
    fn an_entry_takes_the_inline_type_start_when_both_exist() {
        let layout = vec![
            ("Il2CppImage.typeStart".to_string(), 0x18),
            ("Il2CppImage.metadataHandle".to_string(), 0x28),
        ];
        assert!(entry("x", None, 32, &layout).contains("TypeStart::Inline(0x18)"));
    }
}
