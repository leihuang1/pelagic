function test_bindings()
root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
addpath(fullfile(root, 'matlab'));
info = pelagic.version();
assert(strcmp(info.version, strtrim(fileread(fullfile(root, 'matlab', 'VERSION')))));
% Codec round-trips schema arrays (also singleton), singleton objects, empty arrays,
% one-element nested rows and null exactly; Rust remains the only validator.
sample = '{"source_depths_m":[1],"b":[{"c":null}],"d":[],"e":"x\\\"y","speeds_mps":[[1500],[1510]]}';
assert(strcmp(pelagic.internal.encode(pelagic.internal.decode(sample)), sample));
decoded = pelagic.internal.decode('{"receiver_ranges_m":[100,200.5],"layers":[{"x":1}]}');
assert(isequal(decoded.receiver_ranges_m, [100, 200.5]) && iscell(decoded.layers));
assert(strcmp(pelagic.internal.encode(struct('source_depths_m', 25, 'frequency_hz', 50)), ...
    '{"source_depths_m":[25],"frequency_hz":50}'));
assert(strcmp(pelagic.internal.encode(struct('receiver_depths_m', {{10, 40}})), '{"receiver_depths_m":[10,40]}'));
assert(strcmp(pelagic.internal.encode(struct('receiver_depths_m', [10; 40])), '{"receiver_depths_m":[10,40]}'));
assert(pelagic.internal.decode('1820.4999999999998') == 1820.5 - eps(1820.5));
assert(str2double(pelagic.internal.encode(1e-22)) == 1e-22);
assert(isequal(pelagic.internal.decode(pelagic.internal.encode([1, 2; 3, 4])), {[1, 2], [3, 4]}));
try
    pelagic.internal.encode(@sin);
    error('test:MissingError', 'Expected JSON transport failure');
catch exception
    assert(strcmp(exception.identifier, 'pelagic:PG0103'));
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).field, 'json'));
end
[c, load_diagnostics] = pelagic.bellhop.load(fullfile(root, 'examples', 'cases', 'bellhop', 'field-g', 'field-g.json'));
assert(~isempty(load_diagnostics) && strcmp(load_diagnostics{1}.severity, 'warning'));
assert(isnumeric(c.positions.receiver_ranges_m));
assert(max(c.positions.receiver_ranges_m) > 0);
c.positions.source_depths_m = [20, 30];
c.positions.receiver_depths_m = {10, 40, 60};  % older cell spelling still works
c.positions.receiver_ranges_m = [100, 300, 600, 900];
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 3, 4]));
assert(isa(r.pressure, 'single'));
assert(isequal(r.depths_m(:), [10; 40; 60]));
% Compare actual MATLAB interpretation to the same CLI product, with every axis nondegenerate.
verify_case('bellhop', c, r.field);
c.run.receiver_grid = 'irregular';
c.positions.receiver_depths_m = [10, 40, 60, 80];
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 4]));
% A MATLAB scalar for a one-element array field is written as a JSON array.
single = c; single.run.receiver_grid = 'rectilinear'; single.positions.source_depths_m = 25;
r = pelagic.bellhop.run(single);
assert(isequal(size(r.pressure), [1, 4, 4]));
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
c = pelagic.kraken.load(fullfile(root, 'examples', 'cases', 'kraken', 'pekeris', 'pekeris.json'));
c.frequencies{2} = c.frequencies{1};
r = pelagic.kraken.run(c);
assert(numel(r.frequencies) == 2);
assert(isequal(r.frequencies{1}.field.pressure, r.frequencies{2}.field.pressure));
assert(size(r.frequencies{1}.profiles{1}.modes.eigenfunctions, 2) == numel(r.frequencies{1}.profiles{1}.modes.wavenumbers));
previous = getenv('PELAGIC_CLI');
restore = onCleanup(@() setenv('PELAGIC_CLI', previous)); %#ok<NASGU>
if ~ispc
    % The version check runs once per CLI file, not before every load/run.
    real = pelagic.internal.cli();
    if isempty(fileparts(real)), real = strtrim(evalc('system([''command -v '' real])')); end
    scratch = tempname; mkdir(scratch);
    clean_scratch = onCleanup(@() rmdir(scratch, 's')); %#ok<NASGU>
    log = fullfile(scratch, 'calls.log');
    wrapper = fullfile(scratch, 'pelagic-wrapper');
    fid = fopen(wrapper, 'w');
    fprintf(fid, '#!/bin/sh\necho call >> "%s"\nexec "%s" "$@"\n', log, real);
    fclose(fid);
    assert(javaObject('java.io.File', wrapper).setExecutable(true, true));
    setenv('PELAGIC_CLI', wrapper);
    pelagic.version(); pelagic.version();
    pelagic.bellhop.load(fullfile(root, 'examples', 'cases', 'bellhop', 'field-g', 'field-g.json'));
    calls = regexp(strtrim(fileread(log)), '\n', 'split');
    assert(numel(calls) == 2, 'expected one version check and one export, got %d calls', numel(calls));
end
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
