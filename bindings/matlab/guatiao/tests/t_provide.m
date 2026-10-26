% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_provide()
% A library written in M: its folder is the library, its classes the
% providers. What a provider answers is built in the allocator its caller
% carried in, a lent tree grows in the allocator it records, and what it
% was handed is freed.
  cases = {
    'a package folder is its library', @package_library
    'kinds are checked when they are declared', @kind_refusals
    'a provider is checked when its library is made', @provider_refusals
    'what a provider answers lands in the caller''s allocator', @answers_in_carrier
    'a lent map grows in the allocator it records', @lent_map
    'a map handed over is freed, whatever the provider answers', @handed_over
    'a map handed over and kept is all the provider''s own', @kept_handed_over
    'nothing crosses when an argument is refused before the call', @refused_before
    'a call refused at one handed-over argument still takes the others', @refused_handed
    'an instance per configuration, closed when its host destroys it', @configured
    'create builds nothing when it has nowhere to put the instance', @create_null_out
    'a provider says why it cannot run here', @availability
    'a closed library declines a host', @closed_library
  };
end

function reg = preg()
  persistent the
  gt_env('examples');
  if isempty(the) || ~the.is_open()
    the = guatiao.Registry('m-provide', '0.1');
    the.register_library(guatiao.library_of('mlgreeter'));
  end
  reg = the;
end

function package_library()
  gt_env('examples');
  lib = guatiao.library_of('mlgreeter');
  expect_equal(lib.Id, 'mlgreeter');
  expect_equal(lib.Version, '1.0.0');
  names = cellfun(@(p) p.Name, lib.Providers, 'UniformOutput', false);
  expect_equal(names, {'hello', 'shouter'});
  expect_equal(lib.ProviderIds, {'mlgreeter_hello', 'mlgreeter_shouter'});
  expect_equal(lib.Meta.authors, {'jose-pr <jose-pr@coqui.dev>'});
  expect_equal(lib.Meta.license, 'MPL-2.0');
  expect_equal(lib.Meta.summary, 'Greeters Written in M');
  expect_true(strncmp(lib.Meta.description, 'Greeters written in M', 21));
  expect_equal(lib.Meta.urls.homepage, 'https://github.com/Boriken-Dev/guatiao');
  expect_equal(lib.Meta.notices{1}.component, 'mlgreeter');
  expect_true(~isempty(strfind(lib.Meta.notices{1}.text, 'Mozilla Public')));
  again = guatiao.library_of('mlgreeter');
  expect_equal(again.entry(), lib.entry(), 'built once');
  expect_error(@() guatiao.library_of('no_such_package_anywhere'), 'guatiao:NotFound');
  reg = preg();
  p = reg.provider('mlgreeter_hello');
  expect_equal(p.display_name, 'Hello, in M');
  expect_equal(sort(p.kinds), {'counter', 'greeter'});
  q = reg.provider('mlgreeter_shouter');
  expect_equal(q.has_config, true);
  schema = reg.provider_config('mlgreeter_shouter');
  expect_equal(schema.at('required').data(), {'prefix'});
  props = schema.at('properties');
  prefix = props.at('prefix');
  expect_equal(prefix.at('minLength').data(), 1);
  expect_equal(prefix.at('description').data(), 'What every greeting starts with.');
  schema.close();
  expect_equal(reg.provider_config('mlgreeter_hello'), []);
end

function kind_refusals()
  expect_error(@() guatiao.Kind.describe('bad', 1, guatiao.slot('close', {}), 'Object', true), 'guatiao:BadValue');
  expect_error(@() guatiao.Kind.describe('bad', 1, guatiao.slot('f', {}), guatiao.slot('g', {}, 'Required', true)), ...
               'guatiao:BadValue');
  expect_error(@() guatiao.Kind.describe('bad', 1, guatiao.slot('f', {}), guatiao.slot('f', {})), 'guatiao:BadValue');
  expect_error(@() guatiao.slot('f', {'x'}), 'guatiao:BadValue');
  expect_error(@() guatiao.slot('not a name', {}), 'guatiao:BadValue');
  e = guatiao.internal.kinds('of', 'gtt.Pair');
  expect_equal(e.codes{1}.args, [41 41]);
  [~, most] = computer();
  expect_equal(e.size, 8 + (4 + 4 * (most > 2^32)), 'the header and one slot');
  expect_error(@() guatiao.internal.kinds('of', 'gtt.Sizes'), 'guatiao:WrongKind', 'a provider declares no kind');
end

function provider_refusals()
  gt_env('examples');
  expect_error(@() guatiao.Library('lib', '1', {'gtt.Missing'}), 'guatiao:BadValue', 'a required slot left alone');
  expect_error(@() guatiao.Library('lib', '1', {'Ear'}), 'guatiao:BadValue', 'an object kind');
  spec = guatiao.provider('Serves', {'greeterkind.Listener'});
  spec.Class = 'gtt.Picky';
  expect_error(@() guatiao.Library('lib', '1', {spec}), 'guatiao:BadValue', 'serves an object kind');
  expect_error(@() guatiao.Library('lib', '1', {'gtt.Picky', 'gtt.Picky'}), 'guatiao:BadValue', 'two ids');
  expect_error(@() guatiao.Library('lib', '1', {}), 'guatiao:BadValue');
  expect_error(@() guatiao.provider('Config', 1), 'guatiao:BadValue');
end

function answers_in_carrier()
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  reg = preg();
  c = guatiao.Allocator.counting();
  t = reg.provider_table('mlgreeter_hello', 'greeter');
  lib = gt_env('core');
  a = guatiao_mex('alloc_address', c.Id);
  got = guatiao.serving(@() gtfixture('greet_into_alloc', lib, t.table, t.ctx, 'ana', a));
  expect_equal(got, [0 1], 'answered in the carrier''s allocator');
  expect_equal(c.live(), 0, 'freed by the caller');
  got = guatiao.serving(@() gtfixture('greet_into_alloc', lib, t.table, t.ctx, '', a));
  expect_equal(got, [7 0], 'refused');
  expect_equal(c.live(), 0);
  % A value the method built elsewhere is copied into the carrier's allocator.
  reg.register_library(guatiao.Library('m_mirror', '0.1.0', {'gtt.Built'}));
  m = reg.bind('m_mirror_built', 'gtt.Mirror');
  answer = m.show_me();
  expect_equal(answer.data(), struct('built', 'here'));
  answer.close();
end

function lent_map()
  reg = preg();
  c = guatiao.Allocator.counting();
  config = guatiao.Value(struct('name', 'bo'), 'Alloc', c);
  before = c.live();
  g = reg.bind('mlgreeter_hello', 'greeterkind.Greeter');
  expect_equal(g.check(config), true);
  expect_true(c.live() > before, 'grew in its allocator');
  expect_equal(config.data(), struct('name', 'bo', 'greeting', 'hello, bo'));
  config.close();
  expect_equal(c.live(), 0);
end

function handed_over()
  reg = preg();
  c = guatiao.Allocator.counting();
  g = reg.bind('mlgreeter_hello', 'greeterkind.Greeter');
  config = guatiao.Value(struct('name', 'ana'), 'Alloc', c);
  answer = g.greet_from(config);
  answer.close();
  expect_equal(c.live(), 0, 'freed after an answer');
  refused = guatiao.Value(struct('name', ''), 'Alloc', c);
  expect_error(@() g.greet_from(refused), 'guatiao:BadValue');
  expect_equal(c.live(), 0, 'freed after a refusal');
end

function kept_handed_over()
  % Handed over in the caller's allocator, it is moved into this binding's
  % before the method keeps it.
  global GTT_KEPT
  GTT_KEPT = {};
  reg = preg();
  c = guatiao.Allocator.counting();
  reg.register_library(guatiao.Library('m_keeper', '0.1.0', {'gtt.Keeper'}));
  g = reg.bind('m_keeper_keeper', 'greeterkind.Greeter');
  answer = g.greet_from(guatiao.Value(struct('name', 'ivy'), 'Alloc', c));
  expect_equal(answer.data(), struct('greeting', 'kept'), 'it answers');
  answer.close();
  expect_equal(c.live(), 0, 'none of it in the caller''s allocator');
  expect_equal(numel(GTT_KEPT), 1, 'kept');
  expect_equal(GTT_KEPT{1}.data(), struct('name', 'ivy'), 'what it keeps reads');
  GTT_KEPT{1}.close();
  GTT_KEPT = {};
end

function refused_before()
  reg = preg();
  c = guatiao.Allocator.counting();
  reg.register_library(guatiao.Library('m_pair', '0.1.0', {'gtt.Sizes'}));
  p = reg.bind('m_pair_sizes', 'gtt.Pair');
  first = guatiao.Value(struct('a', 1), 'Alloc', c);
  expect_error(@() p.both(first, {1}), 'guatiao:WrongKind');
  expect_equal(first.data(), struct('a', 1), 'nothing crossed');
  expect_equal(p.both(first, struct('b', 2, 'c', 3)), 3);
  expect_true(first.isabsent(), 'handed over');
  expect_equal(c.live(), 0);
end

function refused_handed()
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  reg = preg();
  c = guatiao.Allocator.counting();
  reg.register_library(guatiao.Library('m_pair_refused', '0.1.0', {'gtt.Sizes'}));
  t = reg.provider_table('m_pair_refused_sizes', 'pair');
  owners = guatiao_mex('handles', 1);
  handed = guatiao.Value(struct('b', 2), 'Alloc', c);
  st = guatiao.serving(@() gtfixture('both', t.table, t.ctx, 0, ...
                                     guatiao_mex('node_pointer', handed.owner_id(), {})));
  expect_equal(st, 11, 'refused as the null argument');
  expect_equal(c.live(), 0, 'the map handed over after the null one is freed');
  handed.close();
  % Refused after one was taken: what it was filed under goes too.
  handed = guatiao.Value(struct('a', 1), 'Alloc', c);
  st = guatiao.serving(@() gtfixture('both', t.table, t.ctx, ...
                                     guatiao_mex('node_pointer', handed.owner_id(), {}), 0));
  expect_equal(st, 11, 'refused as the null argument');
  handed.close();
  expect_equal(c.live(), 0);
  expect_equal(guatiao_mex('handles', 1), owners, 'no owner left filed');
  % An object handed over after an argument the gateway refuses is let go.
  reg.register_library(guatiao.Library('m_pocket', '0.1.0', {'gtt.Pockets'}));
  t = reg.provider_table('m_pocket_pockets', 'pocket');
  gone = gtfixture('listeners_destroyed');
  st = guatiao.serving(@() gtfixture('put_refused', t.table, t.ctx));
  expect_equal(st, 7, 'refused as text that is not UTF-8');
  expect_equal(gtfixture('listeners_destroyed'), gone + 1, 'the listener destroyed');
end

function configured()
  global GTT_CLOSED
  GTT_CLOSED = 0;
  reg = preg();
  spec = guatiao.provider('Config', guatiao.Config('label', guatiao.field('string', 'Default', 'x')));
  spec.Class = 'gtt.Closing';
  lib = guatiao.Library('m_closing', '0.1.0', {spec});
  reg.register_library(lib);
  expect_error(@() reg.create('m_closing_closing', struct('label', 1)), 'guatiao:BadValue');
  inst = reg.create('m_closing_closing', struct());
  expect_equal(lib.live_instances(), 1);
  expect_error(@() reg.unload('m_closing'), 'guatiao:WrongKind', 'linked, not mapped');
  if gt_env('fixture')
    expect_equal(gtfixture('library_unload', lib.info()), 14, 'busy');
  end
  g = inst.bind('greeterkind.Greeter');
  answer = g.greet('ana');
  expect_equal(answer.at('greeting').data(), 'x ana');
  inst.close();
  inst.close();
  expect_equal(GTT_CLOSED, 1, 'closed once');
  expect_equal(lib.live_instances(), 0);
  if gt_env('fixture')
    expect_equal(gtfixture('library_unload', lib.info()), 0, 'agrees');
  end
  reg.retire('m_closing');
  expect_true(~any(cellfun(@(p) strcmp(p.id, 'm_closing_closing'), reg.providers())), 'retired');
end

function create_null_out()
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  preg();
  lib = guatiao.library_of('mlgreeter');
  before = guatiao.internal.held('count');
  config = guatiao.Value(struct('prefix', 'hey'));
  st = guatiao.serving(@() gtfixture('create_null_out', gt_env('core'), lib.info(), 1, ...
                                     guatiao_mex('node_pointer', config.owner_id(), {})));
  expect_equal(st, 11);
  expect_equal(guatiao.internal.held('count'), before, 'nothing held');
  config.close();
end

function availability()
  global GTT_WHY
  GTT_WHY = 'not on this machine';
  reg = preg();
  spec = guatiao.provider('Available', @why_not_here);
  spec.Class = 'gtt.Picky';
  reg.register_library(guatiao.Library('m_picky', '0.1.0', {spec}));
  expect_equal(reg.provider_available('m_picky_picky'), struct('available', false, 'reason', 'not on this machine'));
  expect_true(~any(cellfun(@(p) strcmp(p.id, 'm_picky_picky'), reg.available('greeter'))), 'not offered');
  GTT_WHY = [];
  expect_equal(reg.provider_available('m_picky_picky'), struct('available', true, 'reason', ''));
end

function why = why_not_here()
  global GTT_WHY
  why = GTT_WHY;
end

function closed_library()
  reg = preg();
  lib = guatiao.Library('m_declines', '0.1.0', {'gtt.Picky'});
  e = lib.entry();
  lib.close();
  if gt_env('fixture')
    expect_equal(gtfixture('entry_answers', e), false, 'declines');
  end
  reg.register_entry('m_declines', e);
  expect_true(~any(cellfun(@(p) strcmp(p.library, 'm_declines'), reg.providers())), 'nothing registered');
end
