function [c, diagnostics] = load(path, varargin)
% Load ENV+FLP or JSON. Optional arguments: 'flp', path, 'solver', 'krakenc'.
pelagic.version();
args = {'kraken', 'export', char(path)};
assert(mod(numel(varargin), 2) == 0, 'pelagic:PG0002', 'Expected name/value pairs');
for k = 1:2:numel(varargin)
    assert(any(strcmp(varargin{k}, {'flp', 'solver'})), 'pelagic:PG0002', 'Unknown option');
    args = [args, {['--' varargin{k}], char(varargin{k + 1})}]; %#ok<AGROW>
end
[text, diagnostics] = pelagic.internal.invoke(args);
c = pelagic.internal.decode(text);
end
