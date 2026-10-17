// The Mission Control test program of issue #8 in Qt Quick: a window with 300 rows that update, a short animation,
// then idle. The same program exists for every candidate toolkit (see ../README.md); the data is made up.
//   qtquick-bench still     rows, then 20 updates of 30 rows from 3 s, the card arriving at 5 s, then nothing
//   qtquick-bench animate   the card moving without end, and 30 rows changing every 100 ms
// The window is Main.qml; the rows are a model here, and the timers too, as in the other programs.
#include <QAbstractListModel>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QTimer>

#include <algorithm>
#include <cstring>
#include <vector>

static const char *NAMES[12] = {"Help pages", "Test suite",    "Bug reports",  "Website",
                                "Invoices",   "Payments",      "Newsletter",   "Photo archive",
                                "Sales summary", "Translations", "Release notes", "Search index"};
static const char *UNITS[12] = {"pages translated", "tests faster", "reports sorted", "pages lighter",
                                "invoices filed",   "files moved",  "emails ready",   "photos sorted",
                                "rows summed",      "strings done", "notes written",  "files indexed"};

// The made-up state of the rows: every toolkit's program computes the same numbers.
class Rows : public QAbstractListModel {
public:
    enum Role { Mark = Qt::UserRole, Title, Words, Agents, Until };

    Rows() {
        for (int i = 0; i < 300; i++) {
            int total = 100 + (i * 53) % 900;
            rows.push_back({(i * 37) % total, total});
        }
    }

    int rowCount(const QModelIndex &parent = {}) const override { return parent.isValid() ? 0 : int(rows.size()); }

    QVariant data(const QModelIndex &index, int role) const override {
        int i = index.row();
        switch (role) {
        case Mark: return i % 3;
        case Title: return QStringLiteral("%1 %2").arg(QLatin1String(NAMES[i % 12])).arg(i / 12 + 1);
        case Words:
            return QStringLiteral("%1 of %2 %3").arg(rows[i].done).arg(rows[i].total).arg(QLatin1String(UNITS[i % 12]));
        case Agents: return QStringLiteral("%1 agents").arg(1 + (i * 7) % 96);
        case Until: return QStringLiteral("until %1:00").arg(9 + i % 9);
        }
        return {};
    }

    QHash<int, QByteArray> roleNames() const override {
        return {{Mark, "mark"}, {Title, "title"}, {Words, "words"}, {Agents, "agents"}, {Until, "until"}};
    }

    // Tick k changes the 30 rows whose index ends in k % 10.
    void update(int k) {
        for (int i = k % 10; i < int(rows.size()); i += 10) {
            rows[i].done = std::min(rows[i].done + 1 + (i + k) % 3, rows[i].total);
            emit dataChanged(index(i), index(i), {Words});
        }
    }

private:
    struct Data {
        int done, total;
    };
    std::vector<Data> rows;
};

int main(int argc, char *argv[]) {
    bool animate = argc > 1 && !std::strcmp(argv[1], "animate");
    QGuiApplication app(argc, argv);
    QGuiApplication::setDesktopFileName(QStringLiteral("arbor-bench"));
    Rows rows;
    QQmlApplicationEngine engine;
    // animate: the card is there from the start, so it does not arrive first
    engine.setInitialProperties({{"rows", QVariant::fromValue(static_cast<QObject *>(&rows))}, {"shown", animate}});
    engine.loadFromModule("Bench", "Main");
    if (engine.rootObjects().isEmpty())
        return 1;
    QObject *window = engine.rootObjects().first();
    int ticks = 0;
    QTimer updates;
    QObject::connect(&updates, &QTimer::timeout, [&] {
        rows.update(ticks++);
        if (!animate && ticks == 20)
            updates.stop();
    });
    if (animate) {
        QTimer::singleShot(100, [&] { window->setProperty("looping", true); });
        updates.start(100);
    } else {
        QTimer::singleShot(3000, [&] { updates.start(100); });
        QTimer::singleShot(5000, [&] { window->setProperty("shown", true); });
    }
    return app.exec();
}
