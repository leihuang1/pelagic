function info = version()
% Exact CLI/toolbox version match; no silent downloads or upgrades.
% Checked once per CLI file (path, size, modification time), not on every call.
persistent checked
cli = pelagic.internal.cli();
key = identity(cli);
if isstruct(checked) && strcmp(checked.key, key)
    info = checked.info;
    return
end
info = jsondecode(pelagic.internal.invoke({'version', '--json'}));
root = fileparts(fileparts(mfilename('fullpath')));
expected = strtrim(fileread(fullfile(root, 'VERSION')));
assert(strcmp(info.version, expected), 'pelagic:PG0202', ...
    'CLI version %s does not match toolbox %s', info.version, expected);
assert(info.diagnostics_version == 1 && info.case_schema_versions.bellhop == 1 ...
    && info.case_schema_versions.kraken == 1 && info.hdf5_schema_versions.bellhop == 3 ...
    && info.hdf5_schema_versions.kraken == 1, 'pelagic:PG0202', 'Unsupported CLI protocol');
checked = struct('key', key, 'info', info);
end

function key = identity(cli)
% A bare name resolved through PATH has no file to fingerprint; its name is the key.
listing = dir(cli);
if numel(listing) == 1 && ~listing.isdir
    key = sprintf('%s|%d|%.10f', cli, listing.bytes, listing.datenum);
else
    key = cli;
end
end
