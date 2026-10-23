% Your first Pelagic run in MATLAB: transmission loss in the Munk sound channel.
%   addpath('examples/matlab'); example_quickstart     % results go to the current folder
% Writes results/example_quickstart.png in the current folder.
here = fileparts(mfilename('fullpath'));
cases = fullfile(here, '..', 'cases');

c = pelagic.bellhop.load(fullfile(cases, 'bellhop', 'munk-tl', 'munk-tl.json'));
r = pelagic.bellhop.run(c);

% pressure is [source, receiver depth, receiver range]
loss = squeeze(r.field.transmission_loss_db(1, :, :));
loss(~isfinite(loss)) = 120;  % shadow zones: zero pressure, infinite loss
fprintf('%s: %d depths x %d ranges\n', r.title, numel(r.depths_m), numel(r.ranges_m));

figure;
imagesc(r.ranges_m / 1e3, r.depths_m, loss, [60 110]);
colormap(flipud(jet)); colorbar;
xlabel('Range (km)'); ylabel('Depth (m)'); title(r.title);
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_quickstart.png'));
