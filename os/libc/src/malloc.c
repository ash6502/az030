/*
 * malloc: boundary-tag allocator on sbrk.
 *
 * Every block has a header and a footer word holding its size (a multiple of
 * 8, including both tags) with bit 0 set when in use. Free blocks are kept on
 * a doubly linked list (first fit) and coalesced with free neighbours.
 */

#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define TAG 4
#define MIN_BLOCK 16
#define USED 1u

typedef struct free_block {
    unsigned size;
    struct free_block *next, *prev;
} free_block;

static free_block *free_list;
static char *heap_end;          /* address of the epilogue header */

#define SIZE(p) (*(unsigned *)(p) & ~7u)
#define IS_USED(p) (*(unsigned *)(p) & USED)

static void set_tags(char *b, unsigned size, unsigned used)
{
    *(unsigned *)b = size | used;
    *(unsigned *)(b + size - TAG) = size | used;
}

static void unlink_block(free_block *f)
{
    if (f->prev)
        f->prev->next = f->next;
    else
        free_list = f->next;
    if (f->next)
        f->next->prev = f->prev;
}

static void push_block(free_block *f)
{
    f->prev = 0;
    f->next = free_list;
    if (free_list)
        free_list->prev = f;
    free_list = f;
}

/* Grow the heap by at least `need` bytes; returns the new free block. */
static char *grow(unsigned need)
{
    unsigned amount = (need + 0x3FFF) & ~0x3FFFu;
    char *p;
    if (!heap_end) {
        /* prologue footer (in use) + epilogue header (size 0, in use) */
        p = sbrk(8 + 8);
        if (p == (char *)-1)
            return 0;
        p = (char *)(((unsigned)p + 7) & ~7u);
        *(unsigned *)p = 8 | USED;
        heap_end = p + 4;
        *(unsigned *)heap_end = USED;
    }
    p = sbrk(amount);
    if (p == (char *)-1)
        return 0;
    if (p != heap_end + TAG) {
        /* someone else moved the break: start a fresh region */
        p = (char *)(((unsigned)p + 7) & ~7u);
        *(unsigned *)p = 8 | USED;
        heap_end = p + 4;
        amount -= 8;
        p += 4;
    }
    /* the old epilogue header becomes the new block's header */
    {
        char *b = heap_end;
        set_tags(b, amount, 0);
        heap_end = b + amount;
        *(unsigned *)heap_end = USED;
        /* merge with a free block before it */
        {
            unsigned prev = *(unsigned *)(b - TAG);
            if (!(prev & USED)) {
                char *pb = b - (prev & ~7u);
                unlink_block((free_block *)pb);
                set_tags(pb, SIZE(pb) + amount, 0);
                b = pb;
            }
        }
        push_block((free_block *)b);
        return b;
    }
}

void *malloc(size_t n)
{
    unsigned need;
    free_block *f;
    if (n > 0x7FFFFFF0u) {
        errno = ENOMEM;
        return 0;
    }
    need = (n + 2 * TAG + 7) & ~7u;
    if (need < MIN_BLOCK)
        need = MIN_BLOCK;
    for (f = free_list; f; f = f->next)
        if (SIZE(f) >= need)
            break;
    if (!f) {
        f = (free_block *)grow(need);
        if (!f) {
            errno = ENOMEM;
            return 0;
        }
        while (SIZE(f) < need) {
            /* grow merged into a block that is still too small */
            f = (free_block *)grow(need);
            if (!f) {
                errno = ENOMEM;
                return 0;
            }
        }
    }
    unlink_block(f);
    {
        unsigned size = SIZE(f);
        char *b = (char *)f;
        if (size - need >= MIN_BLOCK) {
            set_tags(b, need, USED);
            set_tags(b + need, size - need, 0);
            push_block((free_block *)(b + need));
        } else {
            set_tags(b, size, USED);
        }
        return b + TAG;
    }
}

void free(void *p)
{
    char *b;
    unsigned size;
    if (!p)
        return;
    b = (char *)p - TAG;
    size = SIZE(b);
    /* coalesce with the next block */
    {
        char *nb = b + size;
        if (!IS_USED(nb)) {
            unlink_block((free_block *)nb);
            size += SIZE(nb);
        }
    }
    /* and the previous one */
    {
        unsigned prev = *(unsigned *)(b - TAG);
        if (!(prev & USED)) {
            char *pb = b - (prev & ~7u);
            unlink_block((free_block *)pb);
            size += prev & ~7u;
            b = pb;
        }
    }
    set_tags(b, size, 0);
    push_block((free_block *)b);
}

void *calloc(size_t n, size_t size)
{
    size_t total;
    void *p;
    if (size && n > 0xFFFFFFFFu / size) {
        errno = ENOMEM;
        return 0;
    }
    total = n * size;
    p = malloc(total);
    if (p)
        memset(p, 0, total);
    return p;
}

void *realloc(void *p, size_t n)
{
    char *b;
    unsigned have, need;
    void *q;
    if (!p)
        return malloc(n);
    if (n == 0) {
        free(p);
        return 0;
    }
    b = (char *)p - TAG;
    have = SIZE(b);
    need = (n + 2 * TAG + 7) & ~7u;
    if (need < MIN_BLOCK)
        need = MIN_BLOCK;
    if (need <= have)
        return p;
    /* extend into a free next block */
    {
        char *nb = b + have;
        if (!IS_USED(nb) && have + SIZE(nb) >= need) {
            unsigned total = have + SIZE(nb);
            unlink_block((free_block *)nb);
            if (total - need >= MIN_BLOCK) {
                set_tags(b, need, USED);
                set_tags(b + need, total - need, 0);
                push_block((free_block *)(b + need));
            } else {
                set_tags(b, total, USED);
            }
            return p;
        }
    }
    q = malloc(n);
    if (!q)
        return 0;
    memcpy(q, p, have - 2 * TAG);
    free(p);
    return q;
}

void *aligned_alloc(size_t align, size_t n)
{
    /* blocks are 8-aligned; larger alignments over-allocate (never freed exactly) */
    char *p;
    if (align <= 8)
        return malloc(n);
    p = malloc(n + align);
    if (!p)
        return 0;
    return (void *)(((unsigned)p + align - 1) & ~(align - 1));
}
