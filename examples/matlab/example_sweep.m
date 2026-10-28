% A parameter sweep: change a loaded case for each run and compare the results.
%   addpath('examples/matlab'); example_sweep     % results go to the current folder
% Cases are plain structs, so any field can be changed before running.
% runMany solves them all with one call, sharing the CPUs between them.
% Writes results/example_sweep.png.
here = fileparts(mfilename('fullpath'));
base = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'munk-tl', 'munk-tl.json'));

depths = [100 500 1000 2000];
cases = cell(size(depths));
for k = 1:numel(depths)
    c = base;
    c.title = sprintf('Munk, source at %g m', depths(k));
    c.source.depth = depths(k);   % a scalar is a one-value list
    cases{k} = c;
end
results = pelagic.bellhop.runMany(cases);   % results{k} is what run(cases{k}) returns

figure; hold on;
for k = 1:numel(depths)
    r = results{k};
    row = find(r.depths_m >= 1000, 1);
    plot(r.ranges_m / 1e3, squeeze(r.field.transmission_loss_db(1, row, :)), 'DisplayName', sprintf('source %g m', depths(k)));
end
set(gca, 'YDir', 'reverse'); ylim([50 130]);
xlabel('Range (km)'); ylabel('TL (dB)'); title('Receiver at 1000 m'); legend show;
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_sweep.png'));
