% Range-dependent KRAKEN in the Gulf of Mexico: adiabatic versus coupled modes.
%   addpath('examples/matlab'); example_range_dependent     % results go to the current folder
% The FIELD file chooses the method: gulf.flp (adiabatic) or gulf-coupled.flp.
% Writes results/example_range_dependent.png.
here = fileparts(mfilename('fullpath'));
gulf = fullfile(here, '..', 'cases', 'kraken', 'gulf');

flps = {'gulf.flp', 'gulf-coupled.flp'};
labels = {'adiabatic', 'coupled'};
figure;
for n = 1:2
    c = pelagic.kraken.load(fullfile(gulf, 'gulf.env'), 'flp', fullfile(gulf, flps{n}));
    r = pelagic.kraken.run(c);
    f = r.frequencies{1};
    loss = squeeze(f.field.transmission_loss_db(1, :, :));
    loss(~isfinite(loss)) = 120;
    subplot(2, 1, n);
    imagesc(f.field.ranges_m / 1e3, f.field.depths_m, loss, [60 110]);
    colormap(flipud(jet)); colorbar;
    xlabel('Range (km)'); ylabel('Depth (m)');
    title(sprintf('%s modes, %d profiles', labels{n}, numel(f.profiles)));
end
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_range_dependent.png'));
