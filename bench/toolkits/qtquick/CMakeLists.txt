# The Mission Control test program of issue #8 in Qt Quick (see ../README.md). toolkits/build.sh builds it in a folder
# of its own: cmake -S . -B BUILD -DCMAKE_BUILD_TYPE=Release && cmake --build BUILD
cmake_minimum_required(VERSION 3.21)
project(qtquick_bench LANGUAGES CXX)

find_package(Qt6 6.5 REQUIRED COMPONENTS Quick)
qt_standard_project_setup(REQUIRES 6.5)

qt_add_executable(qtquick-bench main.cpp)
qt_add_qml_module(qtquick-bench URI Bench VERSION 1.0 QML_FILES Main.qml)
target_link_libraries(qtquick-bench PRIVATE Qt6::Quick)
