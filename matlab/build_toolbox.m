function build_toolbox()
% Bundle the already checksum-verified, matching CLI artifacts. No network access.
root = fileparts(mfilename('fullpath'));
repository = fileparts(root);
platforms = {'linux-x64', 'linux-arm64', 'macos-arm64', 'macos-x64', 'windows-x64'};
for k = 1:numel(platforms)
    name = 'pelagic';
    if strcmp(platforms{k}, 'windows-x64'), name = 'pelagic.exe'; end
    assert(isfile(fullfile(root, 'bin', platforms{k}, name)), 'pelagic:PG0001', ...
        'Stage matching CLI artifacts with tools/stage-matlab.py first');
end
stage = tempname;
[ok, message] = mkdir(stage);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve packaging scratch');
cleanup = onCleanup(@() rmdir(stage, 's')); %#ok<NASGU>
copyfile(fullfile(root, '*'), stage);
copyfile(fullfile(repository, 'LICENSE'), fullfile(stage, 'LICENSE'));
copyfile(fullfile(repository, 'NOTICE'), fullfile(stage, 'NOTICE'));
opts = matlab.addons.toolbox.ToolboxOptions(stage, 'afd2e9c4-10dc-4be6-a21b-392078fa6856');
opts.ToolboxName = 'Pelagic';
opts.ToolboxVersion = strtrim(fileread(fullfile(root, 'VERSION')));
opts.AuthorName = 'Pelagic contributors';
opts.Summary = 'BELLHOP and KRAKEN through a version-matched Pelagic CLI';
opts.MinimumMatlabRelease = 'R2023a';
opts.ToolboxMatlabPath = {stage};
opts.SupportedPlatforms.MatlabOnline = false;
if ~isfolder(fullfile(repository, 'target')), mkdir(fullfile(repository, 'target')); end
opts.OutputFile = fullfile(repository, 'target', 'pelagic.mltbx');
matlab.addons.toolbox.packageToolbox(opts);
end
