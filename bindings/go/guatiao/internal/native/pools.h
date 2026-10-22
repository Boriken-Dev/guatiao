/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/* The pools of descriptor slots: declarations only, so the Go file that
 * exports what they call may include it. */

#ifndef GUATIAO_GO_POOLS_H
#define GUATIAO_GO_POOLS_H

#include <stdbool.h>
#include <stdint.h>

int gt_pool_libraries(void);
int gt_pool_providers(void);
void *gt_pool_entry(int i);
void *gt_pool_unload(int i);
void *gt_pool_create(int i);
void *gt_pool_destroy(int i);
void *gt_pool_available(int i);
void *gt_pool_object_destroy(void);

#endif
