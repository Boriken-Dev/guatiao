% This Source Code Form is subject to the terms of the Mozilla Public
% License, v. 2.0. If a copy of the MPL was not distributed with this
% file, You can obtain one at https://mozilla.org/MPL/2.0/.

function cases = t_threads()
% M code runs only on the interpreter's thread, inside a call into
% guatiao. A slot an M provider serves runs when a host calls it there;
% called from any other thread, or with no call into guatiao in progress,
% it answers Unsupported at once, touching no interpreter object, and a
% destroy it is asked for elsewhere waits for the interpreter's thread.
  cases = {
    'an M provider runs on the interpreter''s thread and refuses every other', @threads
    'an object destroyed on another thread is let go on the interpreter''s', @destroy_elsewhere
  };
end

function threads()
  gt_env('examples');
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  reg = guatiao.Registry('m-threads', '0.1');
  reg.register_library(guatiao.library_of('mlgreeter'));
  t = reg.provider_table('mlgreeter_hello', 'greeter');
  lib = gt_env('core');
  here = guatiao.serving(@() gtfixture('greet_here', lib, t.table, t.ctx, 'ana', 'hello, ana', 200));
  expect_equal(here, [200 0 0 0], 'on the interpreter''s thread');
  outside = gtfixture('greet_here', lib, t.table, t.ctx, 'ana', 'hello, ana', 5);
  expect_equal(outside, [0 5 0 13], 'with no call into guatiao in progress');
  away = guatiao.serving(@() gtfixture('greet_threads', lib, t.table, t.ctx, 'ana', 'hello, ana', 8, 250));
  expect_equal(away, [0 2000 0], 'from 8 threads');
  % A thread left running while the interpreter goes on is refused the same way.
  gtfixture('greet_detached', lib, t.table, t.ctx, 'ana', 'hello, ana');
  status = [];
  for i = 1:500
    status = gtfixture('detached');
    if ~isempty(status)
      break
    end
    pause(0.01);
  end
  expect_equal(status, 13, 'the detached thread');
  % Still sound after all that: the provider answers on its thread.
  g = reg.bind('mlgreeter_hello', 'greeterkind.Greeter');
  answer = g.greet('bo');
  expect_equal(answer.data(), struct('greeting', 'hello, bo'));
  reg.close();
end

function destroy_elsewhere()
  gt_env('examples');
  if ~gt_env('fixture')
    skip_case('no compiler built the gtfixture MEX file');
  end
  guatiao.internal.drain();
  before = guatiao.internal.held('count');
  listener = Ear();
  [parts, ~] = guatiao.internal.object_parts('greeterkind.Listener', listener);
  expect_equal(guatiao.internal.held('count'), before + 1, 'held for C');
  gtfixture('destroy_on_thread', parts.table, parts.size, parts.ctx);
  % Queued, not run: the thread that asked could not touch the interpreter.
  expect_equal(guatiao.internal.held('count'), before + 1, 'queued');
  expect_true(~listener.Closed, 'not closed yet');
  guatiao.internal.drain();
  expect_equal(guatiao.internal.held('count'), before, 'let go on this thread');
  expect_true(listener.Closed, 'closed by its holder');
end
