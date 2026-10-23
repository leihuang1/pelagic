% KRAKEN normal modes: mode shapes, dispersion, and the field they build.
%   addpath('examples/matlab'); example_modes     % results go to the current folder
% Writes results/example_modes.png.
here = fileparts(mfilename('fullpath'));
cases = fullfile(here, '..', 'cases');

% A legacy .env is read with its same-stem .flp FIELD file.
c = pelagic.kraken.load(fullfile(cases, 'kraken', 'munk-modes', 'munk-modes.env'));
r = pelagic.kraken.run(c);
f = r.frequencies{1};           % one cell per frequency
m = f.profiles{1}.modes;        % one cell per range profile
fprintf('%d modes at %g Hz\n', numel(m.wavenumbers), f.frequency_hz);

figure;
subplot(2, 2, 1);
plot(real(m.eigenfunctions(:, [1 2 3 10])), m.depths_m);
set(gca, 'YDir', 'reverse'); xlabel('Mode shape'); ylabel('Depth (m)');
legend('1', '2', '3', '10'); title('Mode shapes');
subplot(2, 2, 2);
plot(m.phase_speed_mps, '.'); hold on; plot(m.group_speed_mps, '.');
xlabel('Mode number'); ylabel('Speed (m/s)'); legend('phase', 'group'); title('Dispersion');
subplot(2, 1, 2);
loss = squeeze(f.field.transmission_loss_db(1, :, :));
loss(~isfinite(loss)) = 120;
imagesc(f.field.ranges_m / 1e3, f.field.depths_m, loss, [60 110]);
colormap(flipud(jet)); colorbar; xlabel('Range (km)'); ylabel('Depth (m)'); title('Transmission loss');
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_modes.png'));
