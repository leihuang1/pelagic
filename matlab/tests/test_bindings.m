function test_bindings()
root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
addpath(fullfile(root, 'matlab'));
info = pelagic.version();
assert(strcmp(info.version, '0.3.0'));
% Codec preserves arrays, singleton objects, empty arrays and null without a schema copy.
sample = '{"a":[1],"b":[{"c":null}],"d":[],"e":"x\\\"y"}';
assert(isequaln(jsondecode(jsonencode(pelagic.internal.decode(sample))), jsondecode(sample)));
assert(pelagic.internal.decode('1820.4999999999998') == 1820.5 - eps(1820.5));
assert(str2double(pelagic.internal.encode(1e-22)) == 1e-22);
assert(isequal(pelagic.internal.decode(pelagic.internal.encode([1, 2; 3, 4])), {{1, 2}, {3, 4}}));
try
    pelagic.internal.encode(@sin);
    error('test:MissingError', 'Expected JSON transport failure');
catch exception
    assert(strcmp(exception.identifier, 'pelagic:PG0103'));
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).field, 'json'));
end
[c, load_diagnostics] = pelagic.bellhop.load(fullfile(root, 'examples', 'field-g.json'));
assert(~isempty(load_diagnostics) && strcmp(load_diagnostics{1}.severity, 'warning'));
c.positions.source_depths_m = {20, 30};
c.positions.receiver_depths_m = {10, 40, 60};
c.positions.receiver_ranges_m = {100, 300, 600, 900};
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 3, 4]));
assert(isa(r.pressure, 'single'));
assert(isequal(r.depths_m(:), [10; 40; 60]));
% Compare actual MATLAB interpretation to the same CLI product, with every axis nondegenerate.
verify_case('bellhop', c, r.field);
c.run.receiver_grid = 'irregular';
c.positions.receiver_depths_m = {10, 40, 60, 80};
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 4]));
bad = c; bad.frequency_hz = -1;
try
    pelagic.bellhop.run(bad);
    error('test:MissingError', 'Expected validation failure');
catch exception
    assert(strncmp(exception.identifier, 'pelagic:BH', 10));
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).severity, 'error'));
    assert(strcmp(ds(1).field, 'frequency_hz'));
end
c = pelagic.kraken.load(fullfile(root, 'examples', 'kraken-pekeris.json'));
c.frequencies{2} = c.frequencies{1};
r = pelagic.kraken.run(c);
assert(numel(r.frequencies) == 2);
assert(isequal(r.frequencies{1}.field.pressure, r.frequencies{2}.field.pressure));
assert(size(r.frequencies{1}.profiles{1}.modes.eigenfunctions, 2) == numel(r.frequencies{1}.profiles{1}.modes.wavenumbers));
previous = getenv('PELAGIC_CLI');
restore = onCleanup(@() setenv('PELAGIC_CLI', previous)); %#ok<NASGU>
setenv('PELAGIC_CLI', fullfile(tempname, 'missing-pelagic'));
try
    pelagic.version();
    error('test:MissingError', 'Expected CLI launch failure');
catch exception
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).code, 'PG0001') && strcmp(ds(1).severity, 'error'));
end
fprintf('MATLAB/Octave bindings: PASS\n');
end

function verify_case(model, c, field)
root = tempname; mkdir(root);
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
pelagic.internal.invoke({model, 'run', '-', '--format', 'hdf5', '--output', root}, pelagic.internal.encode(c));
if exist('OCTAVE_VERSION', 'builtin')
    data = builtin('load', '-hdf5', fullfile(root, 'stdin.h5'));
    flat = complex(data.field.pressure_real, data.field.pressure_imaginary);
else
    flat = complex(h5read(fullfile(root, 'stdin.h5'), '/field/pressure_real'), h5read(fullfile(root, 'stdin.h5'), '/field/pressure_imaginary'));
end
for source = 1:numel(field.source_depths_m)
    for depth = 1:numel(field.depths_m)
        for range = 1:numel(field.ranges_m)
            index = ((source - 1) * numel(field.ranges_m) + range - 1) * numel(field.depths_m) + depth;
            assert(field.pressure(source, depth, range) == flat(index));
        end
    end
end
end
