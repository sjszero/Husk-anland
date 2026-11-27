#!/bin/sh
# Guest-side sensor audit. Runs through the Husk loader inside the installed
# rootfs; prints one machine-readable line per observation and exits non-zero on
# any missing/mismatched IIO path or a sensor stream that stopped updating.
set -u

fail=0
fail_msg() {
    fail=$((fail + 1))
    echo "SENSOR_FAIL|$*"
}

is_number() {
    v=$1
    case "$v" in -* ) v=${v#-} ;; esac
    case "$v" in
        ''|.*|*.|*.*.*|*[!0-9.]*) return 1 ;;
        *) return 0 ;;
    esac
}

is_integer() {
    v=$1
    case "$v" in
        ''|-) return 1 ;;
        -*) v=${v#-} ;;
    esac
    case "$v" in ''|*[!0-9]*) return 1 ;; *) return 0 ;; esac
}

check_device() {
    n=$1
    want=$2
    raw_attrs=$3
    required="$raw_attrs"
    case "$n" in
        0) required="$required in_accel_scale sampling_frequency" ;;
        1) required="$required in_magn_scale sampling_frequency" ;;
        2) required="$required in_anglvel_scale sampling_frequency" ;;
        3) required="$required in_illuminance_scale sampling_frequency" ;;
        4) required="$required in_proximity_scale in_proximity_nearlevel sampling_frequency" ;;
    esac

    canonical="/sys/subsystem/iio/devices/iio:device$n"
    for base in /sys/class/iio /sys/bus/iio/devices /sys/subsystem/iio/devices; do
        d="$base/iio:device$n"
        if [ ! -r "$d/name" ]; then
            fail_msg "missing $d/name"
            continue
        fi
        got=$(cat "$d/name") || { fail_msg "unreadable $d/name"; continue; }
        [ "$got" = "$want" ] || fail_msg "$d/name=$got want=$want"
        for attr in $required; do
            if [ ! -r "$d/$attr" ]; then
                fail_msg "missing $d/$attr"
                continue
            fi
            value=$(cat "$d/$attr") || { fail_msg "unreadable $d/$attr"; continue; }
            is_number "$value" || fail_msg "$d/$attr=$value is not numeric"
        done
        echo "SENSOR_PATH|$d|$got"
    done

    sum=0
    for attr in $raw_attrs; do
        [ -r "$canonical/$attr" ] || continue
        value=$(cat "$canonical/$attr") || continue
        is_integer "$value" || continue
        sum=$((sum + value))
    done
    echo "SENSOR_VALUE|$want|$sum"

    # Accelerometer, magnetometer and gyroscope are continuous streams. Light
    # and proximity are Android on-change sensors and may not emit again while
    # their physical value is stable, so requiring a fresh mtime for them would
    # create a false failure.
    case "$n" in
        0|1|2)
            first_raw=${raw_attrs%% *}
            m1=$(stat -c '%y' "$canonical/$first_raw" 2>/dev/null || true)
            sleep 1
            m2=$(stat -c '%y' "$canonical/$first_raw" 2>/dev/null || true)
            echo "SENSOR_MTIME|$want|$m1|$m2"
            if [ -z "$m1" ] || [ "$m1" = "$m2" ]; then
                fail_msg "$want sysfs mtime did not advance"
            fi
            ;;
        *)
            echo "SENSOR_MTIME|$want|on-change"
            ;;
    esac
}

check_device 0 accel_3d 'in_accel_x_raw in_accel_y_raw in_accel_z_raw'
check_device 1 magn_3d 'in_magn_x_raw in_magn_y_raw in_magn_z_raw'
check_device 2 gyro_3d 'in_anglvel_x_raw in_anglvel_y_raw in_anglvel_z_raw'
check_device 3 als 'in_illuminance_raw'
check_device 4 proximity 'in_proximity_raw'

if [ "$fail" -eq 0 ]; then
    echo 'SENSOR_AUDIT|PASS|classes=5|paths=3|stream=live'
    exit 0
fi
echo "SENSOR_AUDIT|FAIL|errors=$fail"
exit 1
