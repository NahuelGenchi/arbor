// The window of the Qt Quick test program (main.cpp). The rows are a ListView, which makes delegates for the rows on
// screen only; the card moves with Animators, which run on the render thread of the scene graph. The words are Qt Quick
// Controls' Label: with Qt 6.10, a plain Text reaches the accessibility bus (AT-SPI) only when it is given
// Accessible.role and Accessible.name, and a Label does without them (toolkits/README.md, Accessibility).
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Window {
    id: root
    required property QtObject rows
    // still: the card arrives once, in 200 ms
    property bool shown: false
    // animate: the card moves up and down without end, 200 ms each way
    property bool looping: false

    width: 960
    height: 640
    visible: true
    title: "arbor bench: Qt Quick"
    color: "#fbfaf7"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 8

        Label {
            text: "3 things need you"
            font.pixelSize: 22
            font.weight: Font.Bold
            color: "#22201c"
        }
        Item {
            Layout.fillWidth: true
            Layout.preferredHeight: 72
            Item {
                width: parent.width
                height: 64
                y: root.shown ? 0 : -40
                opacity: root.shown ? 1 : 0
                Behavior on y { YAnimator { duration: 200; easing.type: Easing.OutCubic } }
                Behavior on opacity { OpacityAnimator { duration: 200; easing.type: Easing.OutCubic } }
                Rectangle {
                    id: card
                    width: parent.width
                    height: parent.height
                    radius: 8
                    color: "#fff3dc"
                    Label {
                        x: 16
                        height: parent.height
                        verticalAlignment: Text.AlignVCenter
                        font.pixelSize: 14
                        color: "#22201c"
                        text: "Payments, moved to version 2: waiting for you. Can be undone."
                    }
                }
                SequentialAnimation {
                    running: root.looping
                    loops: Animation.Infinite
                    ParallelAnimation {
                        YAnimator { target: card; from: 0; to: -40; duration: 200 }
                        OpacityAnimator { target: card; from: 1; to: 0.25; duration: 200 }
                    }
                    ParallelAnimation {
                        YAnimator { target: card; from: -40; to: 0; duration: 200 }
                        OpacityAnimator { target: card; from: 0.25; to: 1; duration: 200 }
                    }
                }
            }
        }
        ListView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.rows
            ScrollBar.vertical: ScrollBar {}
            delegate: Item {
                required property int mark
                required property string title
                required property string words
                required property string agents
                required property string until
                width: ListView.view.width
                height: 32
                Row {
                    x: 16
                    height: parent.height
                    spacing: 12
                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 10
                        height: 10
                        radius: 5
                        color: mark === 1 ? "#a86f0c" : mark === 2 ? "#77736a" : "#2f7d4f"
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 200
                        text: title
                        font.pixelSize: 14
                        font.weight: Font.DemiBold
                        color: "#22201c"
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 320
                        text: words
                        font.pixelSize: 14
                        color: "#22201c"
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 120
                        text: agents
                        font.pixelSize: 14
                        color: "#5c5850"
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        text: until
                        font.pixelSize: 14
                        color: "#5c5850"
                    }
                }
            }
        }
    }
}
