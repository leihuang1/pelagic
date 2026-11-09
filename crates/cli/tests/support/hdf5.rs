//! Compare logical HDF5 contents, not timestamp-bearing container bytes.
use hdf5::types::{VarLenAscii, VarLenUnicode};
use hdf5::{Container, Group, Location, LocationType};

fn values(left: &Container, right: &Container) {
    let name = left.name();
    assert_eq!(left.shape(), right.shape(), "{name}: shape");
    assert_eq!(
        left.space().unwrap().extents().unwrap(),
        right.space().unwrap().extents().unwrap(),
        "{name}: dataspace"
    );
    let dtype = left.dtype().unwrap();
    assert_eq!(dtype, right.dtype().unwrap(), "{name}: type");
    if dtype.is::<f32>() {
        assert_eq!(
            left.read_raw::<f32>()
                .unwrap()
                .into_iter()
                .map(f32::to_bits)
                .collect::<Vec<_>>(),
            right
                .read_raw::<f32>()
                .unwrap()
                .into_iter()
                .map(f32::to_bits)
                .collect::<Vec<_>>(),
            "{name}"
        );
    } else if dtype.is::<f64>() {
        assert_eq!(
            left.read_raw::<f64>()
                .unwrap()
                .into_iter()
                .map(f64::to_bits)
                .collect::<Vec<_>>(),
            right
                .read_raw::<f64>()
                .unwrap()
                .into_iter()
                .map(f64::to_bits)
                .collect::<Vec<_>>(),
            "{name}"
        );
    } else {
        macro_rules! compare {
            ($($ty:ty),*) => { $(if dtype.is::<$ty>() {
                assert_eq!(left.read_raw::<$ty>().unwrap(), right.read_raw::<$ty>().unwrap(), "{name}");
                return;
            })* };
        }
        compare!(
            u8,
            u16,
            u32,
            u64,
            i8,
            i16,
            i32,
            i64,
            bool,
            VarLenUnicode,
            VarLenAscii
        );
        panic!("{name}: unhandled HDF5 type {dtype:?}");
    }
}

fn attributes(left: &Location, right: &Location) {
    let mut names = left.attr_names().unwrap();
    names.sort();
    let mut other = right.attr_names().unwrap();
    other.sort();
    assert_eq!(names, other, "{}: attributes", left.name());
    for name in names {
        values(&left.attr(&name).unwrap(), &right.attr(&name).unwrap());
    }
}

pub fn assert_datasets(left: &Group, right: &Group) {
    attributes(left, right);
    let mut names = left.member_names().unwrap();
    names.sort();
    let mut other = right.member_names().unwrap();
    other.sort();
    assert_eq!(names, other, "{}: members", left.name());
    for name in names {
        let kind = left.loc_type_by_name(&name).unwrap();
        assert_eq!(kind, right.loc_type_by_name(&name).unwrap(), "{name}");
        match kind {
            LocationType::Group => {
                assert_datasets(&left.group(&name).unwrap(), &right.group(&name).unwrap());
            }
            LocationType::Dataset => {
                let left = left.dataset(&name).unwrap();
                let right = right.dataset(&name).unwrap();
                attributes(&left, &right);
                values(&left, &right);
            }
            _ => panic!("{name}: unexpected HDF5 member type {kind:?}"),
        }
    }
}
