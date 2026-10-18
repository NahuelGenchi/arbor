// The Mission Control test program of issue #8 in Flutter: a window with 300 rows that update, a short animation,
// then idle. The same program exists for every candidate toolkit (see ../README.md); the data is made up.
//   flutter-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
//   flutter-bench animate   the card moving without end, and 30 rows changing every 100 ms
// Flutter's widgets layer only. The rows are a ListView.builder, which builds the rows on screen only; the card moves
// with an AnimationController, which the UI thread (Dart) ticks once a frame.
import 'dart:async';
import 'dart:math';

import 'package:flutter/widgets.dart';

const names = [
  'Help pages', 'Test suite', 'Bug reports', 'Website', 'Invoices', 'Payments', 'Newsletter', 'Photo archive',
  'Sales summary', 'Translations', 'Release notes', 'Search index',
];
const units = [
  'pages translated', 'tests faster', 'reports sorted', 'pages lighter', 'invoices filed', 'files moved',
  'emails ready', 'photos sorted', 'rows summed', 'strings done', 'notes written', 'files indexed',
];
const ink = Color(0xFF22201C);
const faint = Color(0xFF5C5850);
const marks = [Color(0xFF2F7D4F), Color(0xFFA86F0C), Color(0xFF77736A)];

void main(List<String> args) => runApp(Bench(animate: args.isNotEmpty && args.first == 'animate'));

class Bench extends StatefulWidget {
  const Bench({super.key, required this.animate});

  final bool animate;

  @override
  State<Bench> createState() => _BenchState();
}

class _BenchState extends State<Bench> with SingleTickerProviderStateMixin {
  // The made-up state of the rows: every toolkit's program computes the same numbers.
  final total = List<int>.generate(300, (i) => 100 + (i * 53) % 900);
  late final done = List<int>.generate(300, (i) => (i * 37) % total[i]);
  final scroll = ScrollController();
  // still: the card arrives once, in 200 ms; animate: it moves up and down without end, 200 ms each way
  late final card = AnimationController(vsync: this, duration: const Duration(milliseconds: 200));
  late final Animation<double> motion =
      widget.animate ? card : CurvedAnimation(parent: card, curve: Curves.easeOut);
  final timers = <Timer>[];
  int ticks = 0;

  @override
  void initState() {
    super.initState();
    if (widget.animate) {
      timers.add(Timer(const Duration(milliseconds: 100), () => card.repeat(reverse: true)));
      timers.add(Timer.periodic(const Duration(milliseconds: 100), (_) => tick()));
    } else {
      timers.add(Timer(const Duration(seconds: 3), () {
        timers.add(Timer.periodic(const Duration(milliseconds: 100), (t) {
          tick();
          if (ticks == 20) t.cancel();
        }));
      }));
      timers.add(Timer(const Duration(seconds: 5), () => card.forward()));
    }
  }

  /// Tick k changes the 30 rows whose index ends in k % 10.
  void tick() {
    setState(() {
      final k = ticks;
      for (var i = k % 10; i < total.length; i += 10) {
        done[i] = min(done[i] + 1 + (i + k) % 3, total[i]);
      }
      ticks++;
    });
  }

  @override
  void dispose() {
    for (final t in timers) {
      t.cancel();
    }
    card.dispose();
    scroll.dispose();
    super.dispose();
  }

  Widget row(BuildContext context, int i) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16),
        child: Row(spacing: 12, children: [
          Container(width: 10, height: 10, decoration: BoxDecoration(color: marks[i % 3], shape: BoxShape.circle)),
          SizedBox(
              width: 200,
              child: Text('${names[i % 12]} ${i ~/ 12 + 1}', style: const TextStyle(fontWeight: FontWeight.w600))),
          SizedBox(width: 320, child: Text('${done[i]} of ${total[i]} ${units[i % 12]}')),
          SizedBox(width: 120, child: Text('${1 + (i * 7) % 96} agents', style: const TextStyle(color: faint))),
          Text('until ${9 + i % 9}:00', style: const TextStyle(color: faint)),
        ]),
      );

  @override
  Widget build(BuildContext context) => WidgetsApp(
        color: const Color(0xFFFBFAF7),
        textStyle: const TextStyle(fontSize: 14, color: ink),
        debugShowCheckedModeBanner: false,
        builder: (context, _) => ColoredBox(
          color: const Color(0xFFFBFAF7),
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, spacing: 8, children: [
              const Text('3 things need you', style: TextStyle(fontSize: 22, fontWeight: FontWeight.w700)),
              SizedBox(
                height: 72,
                child: AnimatedBuilder(
                  animation: motion,
                  builder: (context, child) {
                    final s = motion.value;
                    final (y, alpha) = widget.animate ? (-40 * s, 1 - 0.75 * s) : (-40 * (1 - s), s);
                    return Transform.translate(offset: Offset(0, y), child: Opacity(opacity: alpha, child: child));
                  },
                  child: Container(
                    height: 64,
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    alignment: Alignment.centerLeft,
                    decoration: BoxDecoration(
                        color: const Color(0xFFFFF3DC), borderRadius: BorderRadius.circular(8)),
                    child: const Text('Payments, moved to version 2: waiting for you. Can be undone.'),
                  ),
                ),
              ),
              Expanded(
                child: RawScrollbar(
                  controller: scroll,
                  child: ListView.builder(controller: scroll, itemExtent: 32, itemCount: 300, itemBuilder: row),
                ),
              ),
            ]),
          ),
        ),
      );
}
