function info = version()
% Exact CLI/toolbox version match; no silent downloads or upgrades.
info = jsondecode(pelagic.internal.invoke({'version', '--json'}));
root = fileparts(fileparts(mfilename('fullpath')));
expected = strtrim(fileread(fullfile(root, 'VERSION')));
assert(strcmp(info.version, expected), 'pelagic:PG0202', ...
    'CLI version %s does not match toolbox %s', info.version, expected);
assert(info.diagnostics_version == 1 && info.case_schema_versions.bellhop == 1 ...
    && info.case_schema_versions.kraken == 1 && info.hdf5_schema_versions.bellhop == 3 ...
    && info.hdf5_schema_versions.kraken == 1, 'pelagic:PG0202', 'Unsupported CLI protocol');
end
