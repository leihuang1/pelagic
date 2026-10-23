function names = array_fields()
% JSON members that are numeric arrays in the BELLHOP and KRAKEN v1 case schemas.
% Transport knowledge only, so a MATLAB scalar such as `source_depths_m = 25` still
% encodes as [25]; Rust remains the only validator. python/tests/test_packaging.py
% checks this list against the Rust JSON types and every exported curated fixture.
names = {'attenuation_db_per_wavelength', 'depths_m', 'interface_roughness_m', ...
    'launch_angles_degrees', 'mode_sample_depths_m', 'profile_ranges_m', 'ranges_m', ...
    'receiver_depths_m', 'receiver_offsets_m', 'receiver_ranges_m', 'source_depths_m', ...
    'water_attenuation_db_per_wavelength'};
end
