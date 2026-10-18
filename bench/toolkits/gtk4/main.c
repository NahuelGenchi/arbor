/* The Mission Control test program of issue #8 in GTK 4: a window with 300 rows that update, a short animation, then
 * idle. The same program exists for every candidate toolkit (see ../README.md); the data is made up.
 *   gtk4-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
 *   gtk4-bench animate   the card moving without end, and 30 rows changing every 100 ms
 * The rows are a GtkListView, which makes widgets for the rows on screen only; the card is moved and faded in its
 * snapshot, from a tick callback of the frame clock, so a frame redraws it without laying the window out again. */
#include <gtk/gtk.h>
#include <math.h>
#include <string.h>

static const char *NAMES[12] = { "Help pages", "Test suite", "Bug reports", "Website", "Invoices", "Payments",
                                 "Newsletter", "Photo archive", "Sales summary", "Translations", "Release notes",
                                 "Search index" };
static const char *UNITS[12] = { "pages translated", "tests faster", "reports sorted", "pages lighter",
                                 "invoices filed", "files moved", "emails ready", "photos sorted", "rows summed",
                                 "strings done", "notes written", "files indexed" };
static const char *CSS =
  "window { background-color: #fbfaf7; color: #22201c; font-size: 14px; }"
  ".heading { font-size: 22px; font-weight: 700; }"
  ".card { background-color: #fff3dc; border-radius: 8px; padding: 0 16px; }"
  "listview, listview > row { background: none; padding: 0; min-height: 0; }"
  ".row { min-height: 32px; padding: 0 16px; }"
  ".title { font-weight: 600; }"
  ".faint { color: #5c5850; }"
  ".mark { border-radius: 5px; min-width: 10px; min-height: 10px; }"
  ".mark0 { background-color: #2f7d4f; } .mark1 { background-color: #a86f0c; } .mark2 { background-color: #77736a; }";

static gboolean animate;
static GListStore *rows;
static guint ticks;

/* ---- one row: its made-up state, which every toolkit's program computes the same way ---- */

#define BENCH_TYPE_ITEM (bench_item_get_type())
G_DECLARE_FINAL_TYPE(BenchItem, bench_item, BENCH, ITEM, GObject)
struct _BenchItem {
  GObject parent;
  guint index, done, total;
  char *words;
};
G_DEFINE_FINAL_TYPE(BenchItem, bench_item, G_TYPE_OBJECT)
static GParamSpec *words_property;

static void bench_item_get_property(GObject *o, guint id, GValue *v, GParamSpec *spec) {
  if (id == 1) g_value_set_string(v, BENCH_ITEM(o)->words);
  else G_OBJECT_WARN_INVALID_PROPERTY_ID(o, id, spec);
}

static void bench_item_finalize(GObject *o) {
  g_free(BENCH_ITEM(o)->words);
  G_OBJECT_CLASS(bench_item_parent_class)->finalize(o);
}

static void bench_item_class_init(BenchItemClass *c) {
  G_OBJECT_CLASS(c)->get_property = bench_item_get_property;
  G_OBJECT_CLASS(c)->finalize = bench_item_finalize;
  words_property = g_param_spec_string("words", NULL, NULL, NULL, G_PARAM_READABLE | G_PARAM_STATIC_STRINGS);
  g_object_class_install_property(G_OBJECT_CLASS(c), 1, words_property);
}

static void bench_item_init(BenchItem *item) { (void)item; }

static void set_words(BenchItem *item) {
  g_free(item->words);
  item->words = g_strdup_printf("%u of %u %s", item->done, item->total, UNITS[item->index % 12]);
}

static BenchItem *item_new(guint i) {
  BenchItem *item = g_object_new(BENCH_TYPE_ITEM, NULL);
  item->index = i;
  item->total = 100 + (i * 53) % 900;
  item->done = (i * 37) % item->total;
  set_words(item);
  return item;
}

/* Tick k changes the 30 rows whose index ends in k % 10. */
static gboolean update(gpointer data) {
  (void)data;
  guint k = ticks++;
  for (guint i = k % 10; i < 300; i += 10) {
    BenchItem *item = g_list_model_get_item(G_LIST_MODEL(rows), i);
    item->done = MIN(item->done + 1 + (i + k) % 3, item->total);
    set_words(item);
    g_object_notify_by_pspec(G_OBJECT(item), words_property);
    g_object_unref(item);
  }
  return animate || ticks < 20 ? G_SOURCE_CONTINUE : G_SOURCE_REMOVE;
}

static gboolean begin(gpointer data) {
  (void)data;
  g_timeout_add(100, update, NULL);
  return G_SOURCE_REMOVE;
}

/* ---- the rows of the list ---- */

static GtkWidget *label(const char *css, int width) {
  GtkWidget *l = gtk_label_new(NULL);
  gtk_label_set_xalign(GTK_LABEL(l), 0);
  if (width) gtk_widget_set_size_request(l, width, -1);
  if (css) gtk_widget_add_css_class(l, css);
  return l;
}

static void setup(GtkSignalListItemFactory *f, GtkListItem *li, gpointer data) {
  (void)f, (void)data;
  GtkWidget *row = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 12);
  gtk_widget_add_css_class(row, "row");
  GtkWidget *mark = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
  gtk_widget_add_css_class(mark, "mark");
  gtk_widget_set_valign(mark, GTK_ALIGN_CENTER);
  gtk_box_append(GTK_BOX(row), mark);
  gtk_box_append(GTK_BOX(row), label("title", 200));
  gtk_box_append(GTK_BOX(row), label(NULL, 320));
  gtk_box_append(GTK_BOX(row), label("faint", 120));
  gtk_box_append(GTK_BOX(row), label("faint", 0));
  gtk_list_item_set_child(li, row);
}

static void bind(GtkSignalListItemFactory *f, GtkListItem *li, gpointer data) {
  (void)f, (void)data;
  BenchItem *item = gtk_list_item_get_item(li);
  GtkWidget *mark = gtk_widget_get_first_child(gtk_list_item_get_child(li));
  GtkWidget *title = gtk_widget_get_next_sibling(mark), *words = gtk_widget_get_next_sibling(title);
  GtkWidget *agents = gtk_widget_get_next_sibling(words), *until = gtk_widget_get_next_sibling(agents);
  guint i = item->index;
  char b[64];
  const char *marks[] = { "mark0", "mark1", "mark2", NULL };
  for (int m = 0; marks[m]; m++) gtk_widget_remove_css_class(mark, marks[m]);
  gtk_widget_add_css_class(mark, marks[i % 3]);
  snprintf(b, sizeof b, "%s %u", NAMES[i % 12], i / 12 + 1);
  gtk_label_set_text(GTK_LABEL(title), b);
  g_object_set_data(G_OBJECT(li), "words",
                    g_object_bind_property(item, "words", words, "label", G_BINDING_SYNC_CREATE));
  snprintf(b, sizeof b, "%u agents", 1 + (i * 7) % 96);
  gtk_label_set_text(GTK_LABEL(agents), b);
  snprintf(b, sizeof b, "until %u:00", 9 + i % 9);
  gtk_label_set_text(GTK_LABEL(until), b);
}

static void unbind(GtkSignalListItemFactory *f, GtkListItem *li, gpointer data) {
  (void)f, (void)data;
  GBinding *b = g_object_get_data(G_OBJECT(li), "words");
  if (b) g_binding_unbind(b);
  g_object_set_data(G_OBJECT(li), "words", NULL);
}

/* ---- the card: moved and faded as it is drawn ---- */

#define BENCH_TYPE_CARD (bench_card_get_type())
G_DECLARE_FINAL_TYPE(BenchCard, bench_card, BENCH, CARD, GtkWidget)
struct _BenchCard {
  GtkWidget parent;
  double y, alpha;
  gint64 start;
};
G_DEFINE_FINAL_TYPE(BenchCard, bench_card, GTK_TYPE_WIDGET)

static void bench_card_snapshot(GtkWidget *w, GtkSnapshot *s) {
  BenchCard *c = BENCH_CARD(w);
  gtk_snapshot_save(s);
  gtk_snapshot_translate(s, &GRAPHENE_POINT_INIT(0, (float)c->y));
  gtk_snapshot_push_opacity(s, c->alpha);
  GTK_WIDGET_CLASS(bench_card_parent_class)->snapshot(w, s);
  gtk_snapshot_pop(s);
  gtk_snapshot_restore(s);
}

static void bench_card_dispose(GObject *o) {
  GtkWidget *child;
  while ((child = gtk_widget_get_first_child(GTK_WIDGET(o)))) gtk_widget_unparent(child);
  G_OBJECT_CLASS(bench_card_parent_class)->dispose(o);
}

static void bench_card_class_init(BenchCardClass *c) {
  G_OBJECT_CLASS(c)->dispose = bench_card_dispose;
  GTK_WIDGET_CLASS(c)->snapshot = bench_card_snapshot;
  gtk_widget_class_set_layout_manager_type(GTK_WIDGET_CLASS(c), GTK_TYPE_BIN_LAYOUT);
}

static void bench_card_init(BenchCard *c) {
  GtkWidget *inside = gtk_box_new(GTK_ORIENTATION_HORIZONTAL, 0);
  gtk_widget_add_css_class(inside, "card");
  GtkWidget *text = gtk_label_new("Payments, moved to version 2: waiting for you. Can be undone.");
  gtk_widget_set_hexpand(text, TRUE);
  gtk_label_set_xalign(GTK_LABEL(text), 0);
  gtk_box_append(GTK_BOX(inside), text);
  gtk_widget_set_parent(inside, GTK_WIDGET(c));
  c->alpha = 1;
}

/* still: the card arrives once, in 200 ms; animate: it moves up and down without end, 200 ms each way */
static gboolean move(GtkWidget *w, GdkFrameClock *clock, gpointer data) {
  (void)data;
  BenchCard *c = BENCH_CARD(w);
  gint64 now = gdk_frame_clock_get_frame_time(clock);
  if (!c->start) c->start = now;
  double t = (now - c->start) / 1e6, s;
  gboolean more = TRUE;
  if (animate) {
    double phase = fmod(t, 0.4) / 0.2;
    s = phase <= 1 ? phase : 2 - phase;
    c->y = -40 * s, c->alpha = 1 - 0.75 * s;
  } else {
    double u = MIN(t / 0.2, 1);
    s = 1 - pow(1 - u, 3); /* ease out */
    c->y = -40 * (1 - s), c->alpha = s;
    more = u < 1;
  }
  gtk_widget_queue_draw(w);
  return more ? G_SOURCE_CONTINUE : G_SOURCE_REMOVE;
}

static gboolean start_moving(gpointer card) {
  gtk_widget_add_tick_callback(card, move, NULL, NULL);
  return G_SOURCE_REMOVE;
}

/* ---- the window ---- */

static void activate(GtkApplication *app, gpointer data) {
  (void)data;
  GtkCssProvider *css = gtk_css_provider_new();
  gtk_css_provider_load_from_string(css, CSS);
  gtk_style_context_add_provider_for_display(gdk_display_get_default(), GTK_STYLE_PROVIDER(css),
                                             GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  g_object_unref(css);

  GtkWidget *window = gtk_application_window_new(app);
  gtk_window_set_title(GTK_WINDOW(window), "arbor bench: GTK 4");
  gtk_window_set_default_size(GTK_WINDOW(window), 960, 640);
  GtkWidget *box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 8);
  gtk_widget_set_margin_start(box, 16), gtk_widget_set_margin_end(box, 16);
  gtk_widget_set_margin_top(box, 16), gtk_widget_set_margin_bottom(box, 16);
  GtkWidget *heading = gtk_label_new("3 things need you");
  gtk_widget_add_css_class(heading, "heading");
  gtk_label_set_xalign(GTK_LABEL(heading), 0);
  gtk_box_append(GTK_BOX(box), heading);
  GtkWidget *slot = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0);
  gtk_widget_set_size_request(slot, -1, 72);
  GtkWidget *card = g_object_new(BENCH_TYPE_CARD, NULL);
  gtk_widget_set_size_request(card, -1, 64);
  gtk_box_append(GTK_BOX(slot), card);
  gtk_box_append(GTK_BOX(box), slot);

  rows = g_list_store_new(BENCH_TYPE_ITEM);
  for (guint i = 0; i < 300; i++) {
    BenchItem *item = item_new(i);
    g_list_store_append(rows, item);
    g_object_unref(item);
  }
  GtkListItemFactory *factory = gtk_signal_list_item_factory_new();
  g_signal_connect(factory, "setup", G_CALLBACK(setup), NULL);
  g_signal_connect(factory, "bind", G_CALLBACK(bind), NULL);
  g_signal_connect(factory, "unbind", G_CALLBACK(unbind), NULL);
  GtkWidget *list = gtk_list_view_new(GTK_SELECTION_MODEL(gtk_no_selection_new(G_LIST_MODEL(rows))), factory);
  GtkWidget *scrolled = gtk_scrolled_window_new();
  gtk_scrolled_window_set_child(GTK_SCROLLED_WINDOW(scrolled), list);
  gtk_widget_set_vexpand(scrolled, TRUE);
  gtk_box_append(GTK_BOX(box), scrolled);
  gtk_window_set_child(GTK_WINDOW(window), box);

  if (animate) {
    g_timeout_add(100, start_moving, card);
    g_timeout_add(100, update, NULL);
  } else {
    BENCH_CARD(card)->y = -40, BENCH_CARD(card)->alpha = 0;
    g_timeout_add(3000, begin, NULL);
    g_timeout_add(5000, start_moving, card);
  }
  gtk_window_present(GTK_WINDOW(window));
}

int main(int argc, char **argv) {
  animate = argc > 1 && !strcmp(argv[1], "animate");
  GtkApplication *app = gtk_application_new("arbor.bench", G_APPLICATION_NON_UNIQUE);
  g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
  int status = g_application_run(G_APPLICATION(app), 1, argv); /* the mode is not a file to open */
  g_object_unref(app);
  return status;
}
