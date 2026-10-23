% Arrivals at one receiver: amplitude against travel time (the impulse response).
%   addpath('examples/matlab'); example_arrivals     % results go to the current folder
% Writes results/example_arrivals.png.
here = fileparts(mfilename('fullpath'));
cases = fullfile(here, '..', 'cases');

c = pelagic.bellhop.load(fullfile(cases, 'bellhop', 'munk-arrivals', 'munk-arrivals.env'));
r = pelagic.bellhop.run(c);
a = r.arrivals;
fprintf('%d arrivals between %.3f and %.3f s\n', numel(a.travel_time_s), min(a.travel_time_s), max(a.travel_time_s));
fprintf('travel time (s)   amplitude   surface  bottom\n');
fprintf('%14.4f  %10.3e  %7d  %6d\n', [a.travel_time_s, a.amplitude, double(a.top_bounces), double(a.bottom_bounces)]');

figure;
stem(a.travel_time_s, a.amplitude, 'filled');
xlabel('Travel time (s)'); ylabel('Amplitude'); title(r.title);
if ~isfolder('results'), mkdir('results'); end
print('-dpng', fullfile('results', 'example_arrivals.png'));
