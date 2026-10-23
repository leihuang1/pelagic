% Ray traces: the Munk sound channel and the Gulf of Mexico (.ssp and .bty files).
%   addpath('examples/matlab'); example_rays     % results go to the current folder
% Rays come back as flat columns with zero-based offsets: ray k's points are
% point_offset(k)+1 : point_offset(k+1). Writes results/example_rays.png.
here = fileparts(mfilename('fullpath'));
cases = fullfile(here, '..', 'cases');

names = {'munk-rays', 'gulf'};
figure;
for n = 1:numel(names)
    c = pelagic.bellhop.load(fullfile(cases, 'bellhop', names{n}, [names{n} '.env']));
    r = pelagic.bellhop.run(c);
    rays = r.rays;
    o = rays.point_offset;
    subplot(2, 1, n); hold on;
    for k = 1:numel(o) - 1
        points = (o(k) + 1):o(k + 1);
        if rays.bottom_bounces(k) > 0, colour = 'r'; elseif rays.top_bounces(k) > 0, colour = 'b'; else, colour = 'k'; end
        plot(rays.range_m(points) / 1e3, rays.depth_m(points), colour);
    end
    set(gca, 'YDir', 'reverse');
    xlabel('Range (km)'); ylabel('Depth (m)');
    title(sprintf('%s (%d rays)', r.title, numel(o) - 1));
end
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_rays.png'));
