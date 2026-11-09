#[path = "support/hdf5.rs"]
mod comparison;

use hdf5::types::VarLenUnicode;
use hdf5::{File, Group};
use std::fs;

fn text(value: &str) -> VarLenUnicode {
    value.parse().unwrap()
}

fn contents(file: &File) {
    file.new_attr::<VarLenUnicode>()
        .shape(())
        .create("input_sha256")
        .unwrap()
        .write_scalar(&text("provenance"))
        .unwrap();
    file.new_attr::<VarLenUnicode>()
        .shape(2)
        .create("warnings")
        .unwrap()
        .write_raw(&[text("first"), text("second")])
        .unwrap();
    let group = file.create_group("nested").unwrap();
    group
        .new_dataset_builder()
        .with_data(&[0.0_f64, -0.0])
        .create("data")
        .unwrap();
    group
        .new_dataset_builder()
        .with_data(&[u64::MAX])
        .create("integer")
        .unwrap();
    group
        .dataset("data")
        .unwrap()
        .new_attr::<VarLenUnicode>()
        .create("unit")
        .unwrap()
        .write_scalar(&text("m"))
        .unwrap();
}

#[test]
fn comparison_checks_provenance_array_attributes_shapes_types_and_exact_values() {
    let root = std::env::temp_dir().join(format!("pelagic-hdf5-comparison-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let left = File::create(root.join("left.h5")).unwrap();
    contents(&left);
    let changes: [fn(&Group); 8] = [
        |g| {
            g.attr("input_sha256")
                .unwrap()
                .write_scalar(&text("changed"))
                .unwrap();
        },
        |g| {
            g.attr("warnings")
                .unwrap()
                .write_raw(&[text("first"), text("changed")])
                .unwrap();
        },
        |g| {
            g.dataset("nested/data")
                .unwrap()
                .write_raw(&[0.0_f64, 0.0])
                .unwrap();
        },
        |g| {
            g.dataset("nested/integer")
                .unwrap()
                .write_raw(&[u64::MAX - 1])
                .unwrap();
        },
        |g| {
            g.dataset("nested/data")
                .unwrap()
                .attr("unit")
                .unwrap()
                .write_scalar(&text("km"))
                .unwrap();
        },
        |g| {
            g.unlink("nested/integer").unwrap();
            g.new_dataset::<u64>()
                .shape((1, 1))
                .create("nested/integer")
                .unwrap()
                .write_raw(&[u64::MAX])
                .unwrap();
        },
        |g| {
            g.unlink("nested/integer").unwrap();
            g.new_dataset_builder()
                .with_data(&[1_i64])
                .create("nested/integer")
                .unwrap();
        },
        |g| g.unlink("nested").unwrap(),
    ];
    for (index, change) in changes.into_iter().enumerate() {
        let right = File::create(root.join("right.h5")).unwrap();
        contents(&right);
        comparison::assert_datasets(&left, &right);
        change(&right);
        assert!(
            std::panic::catch_unwind(|| comparison::assert_datasets(&left, &right)).is_err(),
            "missed change {index}"
        );
    }
    drop(left);
    fs::remove_dir_all(root).unwrap();
}
