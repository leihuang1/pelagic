function test_bindings()
root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
addpath(fullfile(root, 'matlab'));
info = pelagic.version();
assert(strcmp(info.version, strtrim(fileread(fullfile(root, 'matlab', 'VERSION')))));
% Codec round-trips scalars, singleton objects, empty arrays, table rows (including
% one-value rows) and null exactly; Rust remains the only validator. A one-value list
% such as [1] comes back as the number 1, which every case-JSON list accepts.
sample = '{"depth":1,"b":[{"c":null}],"d":[],"e":"x\\\"y","ssp":[[0,1500],[100]]}';
assert(strcmp(pelagic.internal.encode(pelagic.internal.decode(sample)), sample));
decoded = pelagic.internal.decode('{"range":[100,200.5],"media":[{"to":1}]}');
assert(isequal(decoded.range, [100, 200.5]) && iscell(decoded.media));
assert(strcmp(pelagic.internal.encode(struct('depth', 25, 'frequency', 50)), ...
    '{"depth":25,"frequency":50}'));
assert(strcmp(pelagic.internal.encode(struct('depth', {{10, 40}})), '{"depth":[10,40]}'));
assert(strcmp(pelagic.internal.encode(struct('depth', [10; 40])), '{"depth":[10,40]}'));
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
assert(isstruct(c.receivers.range) && c.receivers.range.count > 0);
c.source.depth = [20, 30];
c.receivers.depth = {10, 40, 60};  % cell spelling still works
c.receivers.range = [100, 300, 600, 900];
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 3, 4]));
assert(isa(r.pressure, 'single'));
assert(isequal(r.depths_m(:), [10; 40; 60]));
% Compare actual MATLAB interpretation to the same CLI product, with every axis nondegenerate.
verify_case('bellhop', c, r.field);
c.receivers.grid = 'irregular';
c.receivers.depth = [10, 40, 60, 80];
r = pelagic.bellhop.run(c);
assert(isequal(size(r.pressure), [2, 4]));
% One range with several sources and depths: the axes come from one source's samples.
one_range = c; one_range.receivers.grid = 'rectilinear'; one_range.receivers.range = 500;
r = pelagic.bellhop.run(one_range);
assert(isequal(size(r.pressure), [2, 4]) && isequal(r.ranges_m, 500) && numel(r.depths_m) == 4);
verify_case('bellhop', one_range, r.field);
% A MATLAB scalar is a one-value list.
single = c; single.receivers.grid = 'rectilinear'; single.source.depth = 25;
r = pelagic.bellhop.run(single);
assert(isequal(size(r.pressure), [1, 4, 4]));
bad = c; bad.frequency = -1;
try
    pelagic.bellhop.run(bad);
    error('test:MissingError', 'Expected validation failure');
catch exception
    assert(strncmp(exception.identifier, 'pelagic:BH', 10));
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).severity, 'error'));
    assert(strcmp(ds(1).field, 'frequency'));
end
% The thread count never changes results; invalid counts are adapter errors.
one = pelagic.bellhop.run(c, 'Threads', 1);
three = pelagic.bellhop.run(c, 'threads', 3);
assert(isequal(one.pressure, three.pressure));
for invalid = {0, 1.5, Inf, 'two', [1, 2]}
    try
        pelagic.bellhop.run(c, 'Threads', invalid{1});
        error('test:MissingError', 'Expected an invalid thread count');
    catch exception
        assert(strcmp(exception.identifier, 'pelagic:PG0002'));
    end
end
c = pelagic.kraken.load(fullfile(root, 'examples', 'cases', 'kraken', 'pekeris', 'pekeris.json'));
c.frequency = [c.frequency, c.frequency];
r = pelagic.kraken.run(c);
assert(numel(r.frequencies) == 2);
assert(isequal(r.frequencies{1}.field.pressure, r.frequencies{2}.field.pressure));
two = pelagic.kraken.run(c, 'Threads', 2);
assert(isequal(two.frequencies{1}.field.pressure, r.frequencies{1}.field.pressure));
assert(size(r.frequencies{1}.profiles{1}.modes.eigenfunctions, 2) == numel(r.frequencies{1}.profiles{1}.modes.wavenumbers));
% runMany solves every case in one CLI call; each result is what run returns.
field_g = fullfile(root, 'examples', 'cases', 'bellhop', 'field-g', 'field-g.json');
base = pelagic.bellhop.load(field_g);
deeper = base; deeper.source.depth = 40;
bad = base; bad.frequency = -1;
missing = fullfile(tempname, 'missing.json');
cases = {base, field_g, deeper, bad, missing};
[many, failures] = pelagic.bellhop.runMany(cases, 'Workers', 2, 'Threads', 1);
assert(isequal(size(many), size(cases)) && isempty(many{4}) && isempty(many{5}));
same_result(many{1}, pelagic.bellhop.run(base));
loaded = pelagic.bellhop.load(field_g);
same_result(many{2}, pelagic.bellhop.run(loaded));
same_result(many{3}, pelagic.bellhop.run(deeper));
assert(isequal([failures.index], [4, 5]));
assert(strcmp(failures(1).code, 'BH0201') && strcmp(failures(1).diagnostics{1}.field, 'frequency'));
assert(strcmp(failures(2).code, 'BH0001') && strcmp(failures(2).diagnostics{1}.field, 'input'));
% Any shape of cell array works; a failed column does not hold back the others.
[grid, failures] = pelagic.bellhop.runMany({missing, base; missing, deeper});
assert(isequal(size(grid), [2, 2]) && isempty(grid{1, 1}) && isempty(grid{2, 1}));
assert(isequal(grid{1, 2}.pressure, many{1}.pressure) && isequal(grid{2, 2}.pressure, many{3}.pressure));
assert(isequal([failures.index], [1, 2]));
% With one output, the first failure is thrown after every case has finished.
try
    many = pelagic.bellhop.runMany({base, bad, bad});
    error('test:MissingError', 'Expected a runMany failure');
catch exception
    assert(strcmp(exception.identifier, 'pelagic:BH0201'));
    ds = pelagic.diagnostics(exception);
    assert(strcmp(ds(1).field, 'frequency'));
    payload = jsondecode(exception.message);
    assert(payload.index == 2 && payload.failure_count == 2);
end
% 'Output' keeps each case's files; a later call into it reports only its own cases.
kept = tempname;
clean_kept = onCleanup(@() rmdir(kept, 's')); %#ok<NASGU>
first = pelagic.bellhop.runMany({base, deeper}, 'Output', kept);
assert(isfile(fullfile(kept, 'c00001', 'c00001.h5')) && isfile(fullfile(kept, 'c00002', 'c00002.h5')));
assert(isfile(fullfile(kept, 'status.ndjson')));
assert(isequal(first{2}.pressure, many{3}.pressure));
folder = kept;
if ~exist('OCTAVE_VERSION', 'builtin'), folder = string(kept); end  % Octave has no strings
[again, failures] = pelagic.bellhop.runMany({bad, deeper}, 'Output', folder);
assert(isempty(again{1}) && isequal([failures.index], 1) && isequal(again{2}.pressure, many{3}.pressure));
[none, failures] = pelagic.bellhop.runMany({});
assert(isempty(none) && isempty(failures));
invalid = {{'not a cell'}, {{1}}, {{base, {}}}, {{''}}, {{base}, 'Workers', 0}, ...
    {{base}, 'Threads', 1.5}, {{base}, 'Output'}, {{base}, 'Output', 5}, {{base}, 'Bogus', 1}};
for k = 1:numel(invalid)
    args = invalid{k};
    try
        pelagic.bellhop.runMany(args{:});
        error('test:MissingError', 'Expected invalid runMany arguments');
    catch exception
        assert(strcmp(exception.identifier, 'pelagic:PG0002'), 'invalid arguments %d: %s', k, exception.identifier);
    end
end
pekeris = fullfile(root, 'examples', 'cases', 'kraken', 'pekeris', 'pekeris.env');
[kr, failures] = pelagic.kraken.runMany({c, pekeris}, 'Threads', 2);
assert(isempty(failures));
same_result(kr{1}, pelagic.kraken.run(c));
legacy = pelagic.kraken.load(pekeris);
same_result(kr{2}, pelagic.kraken.run(legacy));
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
    % A case the CLI never reports fails, though an earlier call into the same
    % Output folder recorded it as solved.
    stopper = fullfile(scratch, 'pelagic-stopper');
    fid = fopen(stopper, 'w');
    fprintf(fid, '#!/bin/sh\nfor a in "$@"; do if [ "$a" = run-many ]; then exit 3; fi; done\nexec "%s" "$@"\n', real);
    fclose(fid);
    assert(javaObject('java.io.File', stopper).setExecutable(true, true));
    setenv('PELAGIC_CLI', stopper);
    [stale, failures] = pelagic.bellhop.runMany({base, deeper}, 'Output', kept);
    assert(isempty(stale{2}) && isequal([failures.index], [1, 2]) && strcmp(failures(2).code, 'PG0001'));
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

function same_result(actual, expected)
% Diagnostics differ only in source.path: run reads stdin, runMany a file.
assert(isequal(rmfield(actual, 'diagnostics'), rmfield(expected, 'diagnostics')));
assert(numel(actual.diagnostics) == numel(expected.diagnostics));
for k = 1:numel(actual.diagnostics)
    a = actual.diagnostics{k}; e = expected.diagnostics{k};
    assert(isequal(rmfield(a, 'source'), rmfield(e, 'source')));
end
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
