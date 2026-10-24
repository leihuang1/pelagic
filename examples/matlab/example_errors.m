% Errors: every problem has a stable code and names the field involved.
%   addpath('examples/matlab'); example_errors     % results go to the current folder
% Match on the code (the part after 'pelagic:'), not the message text.
here = fileparts(mfilename('fullpath'));
c = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'munk-tl', 'munk-tl.json'));

c.frequency = -50;
try
    pelagic.bellhop.run(c);
catch err
    fprintf('identifier: %s\n', err.identifier);
    d = pelagic.diagnostics(err);     % all problems, as a struct array
    for k = 1:numel(d)
        fprintf('%s at %s: %s\n', d(k).code, d(k).field, d(k).message);
    end
end

% Load warnings are returned as a second output (a cell array) instead of stopping the load.
[~, warnings] = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'field-g', 'field-g.json'));
for k = 1:numel(warnings)
    fprintf('warning %s: %s\n', warnings{k}.code, warnings{k}.message);
end
