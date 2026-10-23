function test_examples()
% Run every script in examples/matlab with figures hidden, in a scratch folder.
root = fileparts(fileparts(fileparts(mfilename('fullpath'))));
addpath(fullfile(root, 'matlab'));
addpath(fullfile(root, 'examples', 'matlab'));
scripts = dir(fullfile(root, 'examples', 'matlab', 'example_*.m'));
assert(numel(scripts) >= 8, 'expected the example scripts');
work = tempname;
mkdir(work);
previous = cd(work);
restore = onCleanup(@() cd(previous)); %#ok<NASGU>
visible = get(0, 'DefaultFigureVisible');
set(0, 'DefaultFigureVisible', 'off');
reset = onCleanup(@() set(0, 'DefaultFigureVisible', visible)); %#ok<NASGU>
for k = 1:numel(scripts)
    fprintf('running %s\n', scripts(k).name);
    run_script(scripts(k).name(1:end - 2));
    close all;
end
images = dir(fullfile(work, 'results', '*.png'));
assert(numel(images) >= numel(scripts) - 1, 'each plotting example should write a PNG');
fprintf('test_examples: %d scripts PASS\n', numel(scripts));
end

function run_script(name)
% A separate workspace, so scripts cannot overwrite each other's variables. Calling
% the script by name (not run()) keeps the current folder, as for users.
eval(name);
end
