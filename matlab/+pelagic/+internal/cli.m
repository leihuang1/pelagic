function cli = cli()
% Resolve the CLI: PELAGIC_CLI, then the bundled platform binary, then pelagic on PATH.
cli = getenv('PELAGIC_CLI');
if ~isempty(cli), return; end
base = fileparts(fileparts(fileparts(mfilename('fullpath'))));
if ispc, platform = 'windows-x64'; name = 'pelagic.exe';
elseif ismac
    if ~isempty(strfind(computer, 'A64')) || ~isempty(strfind(computer, 'aarch64')), platform = 'macos-arm64';
    else, platform = 'macos-x64'; end
    name = 'pelagic';
else
    if ~isempty(strfind(computer, 'aarch64')) || ~isempty(strfind(computer, 'arm64')), platform = 'linux-arm64';
    else, platform = 'linux-x64'; end
    name = 'pelagic';
end
cli = fullfile(base, 'bin', platform, name);
if ~isfile(cli), cli = name;
elseif ~ispc
    % Toolbox archives discard Unix execute bits; repair only our bundled binary.
    file = javaObject('java.io.File', cli);
    assert(file.canExecute() || file.setExecutable(true, true), 'pelagic:PG0001', ...
        'Cannot make bundled CLI executable; set PELAGIC_CLI to an installed binary');
end
end
