function test_curated()
% Exercise the same committed inputs as Python, comparing pre/post MATLAB JSON transport.
root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
addpath(fullfile(root, 'matlab'));
state = warning; warning('off', 'all');
restore = onCleanup(@() warning(state)); %#ok<NASGU>
successes = 0; rejections = 0;
for model = {'bellhop', 'kraken'}
    name = model{1};
    folder = fullfile(root, 'crates', name, 'tests', 'fixtures');
    engines = {''};
    if strcmp(name, 'bellhop'), folder = fullfile(folder, 'golden');
    else, engines = {'kraken', 'krakenc'}; end
    cases = dir(fullfile(folder, '*.env'));
    for engine = engines
        for k = 1:numel(cases)
            args = {name, 'export', fullfile(folder, cases(k).name)};
            if ~isempty(engine{1}), args = [args, {'--solver', engine{1}}]; end %#ok<AGROW>
            try
                text = pelagic.internal.invoke(args);
            catch exception
                assert_model_error(exception);
                rejections = rejections + 1;
                continue
            end
            c = pelagic.internal.decode(text);
            [ok, expected, error_id] = baseline(name, text, c);
            try
                actual = pelagic.internal.run(name, c);
            catch exception
                if ok, rethrow(exception); end
                assert(strcmp(exception.identifier, error_id));
                rejections = rejections + 1;
                continue
            end
            assert(ok, 'MATLAB accepted a reference-rejected input');
            actual = rmfield(actual, {'provenance', 'diagnostics'});
            assert(isequaln(actual, expected), ['MATLAB JSON transport changed result: ' cases(k).name]);
            successes = successes + 1;
        end
    end
end
assert(successes >= 100, 'Curated test selection unexpectedly empty or rejected');
fprintf('MATLAB/Octave curated: %d complete matches, %d matched/loader rejections\n', successes, rejections);
end

function [ok, result, error_id] = baseline(model, text, c)
root = tempname; mkdir(root);
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
ok = false; result = []; error_id = '';
try
    pelagic.internal.invoke({model, 'run', '-', '--format', 'hdf5', '--output', root}, text);
catch exception
    assert_model_error(exception);
    error_id = exception.identifier;
    return
end
result = pelagic.internal.read_result(model, fullfile(root, 'stdin.h5'), c);
ok = true;
end

function assert_model_error(exception)
if ~(strncmp(exception.identifier, 'pelagic:BH', 10) || strncmp(exception.identifier, 'pelagic:KR', 10))
    rethrow(exception);
end
end
