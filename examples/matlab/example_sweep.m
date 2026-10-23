% A parameter sweep: change a loaded case in a loop and compare the results.
%   addpath('examples/matlab'); example_sweep     % results go to the current folder
% Cases are plain structs, so any field can be changed before running.
% Writes results/example_sweep.png.
here = fileparts(mfilename('fullpath'));
base = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'munk-tl', 'munk-tl.json'));

depths = [100 500 1000 2000];
figure; hold on;
for d = depths
    c = base;
    c.title = sprintf('Munk, source at %g m', d);
    c.positions.source_depths_m = d;   % a scalar is sent as a one-element array
    r = pelagic.bellhop.run(c);
    row = find(r.depths_m >= 1000, 1);
    plot(r.ranges_m / 1e3, squeeze(r.field.transmission_loss_db(1, row, :)), 'DisplayName', sprintf('source %g m', d));
end
set(gca, 'YDir', 'reverse'); ylim([50 130]);
xlabel('Range (km)'); ylabel('TL (dB)'); title('Receiver at 1000 m'); legend show;
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_sweep.png'));
