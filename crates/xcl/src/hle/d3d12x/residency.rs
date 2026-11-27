//! Residency priorities of XCL's video memory. Past the budget Windows gives the process, the
//! video memory manager moves allocations to system memory by their residency priority (all
//! NORMAL by default), and the GPU reads what was moved over PCIe. A retail title's world on a
//! 10 GB card wants ~15 GB: what goes over should be what a frame touches least, not the
//! render targets it writes every frame.
//!
//! `XCL_RESIDENCY=<class>:<priority>,…` sets a priority for a class of XCL's heaps as they are
//! made: classes `pages` (pool heaps of title render targets and textures, `vram`), `arena`
//! (render targets that cannot be tiled), `sparse` (video-memory tiles of the title's sparse
//! windows), `committed` (the title's committed resources); priorities `min`, `low`, `normal`,
//! `high`, `max`. Unset: none is set (as before).
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D12::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Pages,
    Arena,
    Sparse,
    Committed,
}

const CLASSES: [(&str, Class); 4] = [("pages", Class::Pages), ("arena", Class::Arena), ("sparse", Class::Sparse), ("committed", Class::Committed)];

fn priority_of(name: &str) -> Option<D3D12_RESIDENCY_PRIORITY> {
    Some(match name {
        "min" => D3D12_RESIDENCY_PRIORITY_MINIMUM,
        "low" => D3D12_RESIDENCY_PRIORITY_LOW,
        "normal" => D3D12_RESIDENCY_PRIORITY_NORMAL,
        "high" => D3D12_RESIDENCY_PRIORITY_HIGH,
        "max" => D3D12_RESIDENCY_PRIORITY_MAXIMUM,
        _ => return None,
    })
}

/// The priorities of `XCL_RESIDENCY`'s value (entries that are not `<class>:<priority>` are passed over).
fn parse(v: &str) -> Vec<(Class, D3D12_RESIDENCY_PRIORITY)> {
    v.split(',')
        .filter_map(|e| {
            let (c, p) = e.split_once(':')?;
            let class = CLASSES.iter().find(|(n, _)| n.eq_ignore_ascii_case(c.trim()))?.1;
            Some((class, priority_of(&p.trim().to_ascii_lowercase())?))
        })
        .collect()
}

fn priority(class: Class) -> Option<D3D12_RESIDENCY_PRIORITY> {
    static P: std::sync::OnceLock<Vec<(Class, D3D12_RESIDENCY_PRIORITY)>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let p = std::env::var("XCL_RESIDENCY").map(|v| parse(&v)).unwrap_or_default();
        if !p.is_empty() {
            crate::xlog_info!("D3D12.X: residency priorities (XCL_RESIDENCY): {:?}", p.iter().map(|(c, r)| (c, format!("{:#x}", r.0))).collect::<Vec<_>>());
        }
        p
    })
    .iter()
    .find(|(c, _)| *c == class)
    .map(|p| p.1)
}

/// Give `object` (a heap or committed resource of `dev`) its class's priority, when one is set.
pub(super) unsafe fn set(dev: &ID3D12Device, object: &impl Interface, class: Class) {
    let Some(p) = priority(class) else { return };
    let (Ok(dev1), Ok(pageable)) = (dev.cast::<ID3D12Device1>(), object.cast::<ID3D12Pageable>()) else { return };
    if let Err(e) = dev1.SetResidencyPriority(1, &Some(pageable), &p) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| crate::xlog_warn!("D3D12.X: SetResidencyPriority ({class:?}): {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_priorities_of_the_switch_are_read_by_class() {
        assert_eq!(
            parse("pages:max, Sparse:low,committed:NORMAL"),
            vec![(Class::Pages, D3D12_RESIDENCY_PRIORITY_MAXIMUM), (Class::Sparse, D3D12_RESIDENCY_PRIORITY_LOW), (Class::Committed, D3D12_RESIDENCY_PRIORITY_NORMAL)]
        );
        // unknown classes and priorities, and entries without both, are passed over
        assert_eq!(parse("textures:max,arena:huge,arena,arena:high"), vec![(Class::Arena, D3D12_RESIDENCY_PRIORITY_HIGH)]);
        assert!(parse("").is_empty());
    }
}

/// LOCAL PROTOTYPE: the CPU page property of the system-memory pool heaps (`XCL_SPARSE_SYSMEM`,
/// `XCL_TEXTURE_SYSMEM`): `XCL_SYS_HEAP_CPU=none` (the CPU cannot map them – nothing for a capture
/// layer to shadow), `wc` (write-combined), else write-back.
pub(super) fn sys_page_property() -> D3D12_CPU_PAGE_PROPERTY {
    match std::env::var("XCL_SYS_HEAP_CPU").as_deref().map(str::trim) {
        Ok("none") => D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE,
        Ok("wc") => D3D12_CPU_PAGE_PROPERTY_WRITE_COMBINE,
        _ => D3D12_CPU_PAGE_PROPERTY_WRITE_BACK,
    }
}
