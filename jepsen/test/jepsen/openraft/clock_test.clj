(ns jepsen.openraft.clock-test
  (:require [clojure.test :refer [deftest is]]
            [jepsen.control :as c]
            [jepsen.openraft.clock :as clock]))

(deftest initializes-a-missing-node-local-clock
  (let [calls (atom [])]
    (with-redefs [c/exec (fn [& command]
                           (swap! calls conj command)
                           (if (= :bash (first command))
                             (nth command 4)
                             ""))]
      (is (= clock/normal-setting (clock/prepare!))))
    (is (= :mkdir (ffirst @calls)))
    (is (= :bash (first (second @calls))))
    (is (re-find #"test ! -e" (nth (second @calls) 2)))
    (is (= clock/normal-setting (nth (second @calls) 4)))
    (is (= clock/control-file (nth (second @calls) 5)))))

(deftest application-clock-excludes-monotonic-time
  (is (= clock/library (:LD_PRELOAD clock/application-env)))
  (is (= clock/control-file
         (:FAKETIME_TIMESTAMP_FILE clock/application-env)))
  (is (= "1" (:NO_FAKE_STAT clock/application-env)))
  (is (not (contains? clock/application-env :FAKETIME_XRESET)))
  (is (not (contains? clock/application-env :FAKETIME_DONT_FAKE_STAT)))
  (is (= "1" (:FAKETIME_DONT_FAKE_MONOTONIC clock/application-env))))

(deftest reads-and-probes-clock-state-externally
  (let [calls (atom [])]
    (with-redefs [c/exec (fn [& command]
                           (swap! calls conj command)
                           (if (= :cat (first command))
                             clock/normal-setting
                             "1700000000123"))]
      (is (= clock/normal-setting (clock/read-setting!)))
      (is (= 1700000000123 (clock/probe-wall-time-ms!))))
    (is (= [:cat clock/control-file] (first @calls)))
    (is (some #{:date} (second @calls)))))

(deftest verifies-the-application-preload-from-proc
  (let [call (atom nil)]
    (with-redefs [c/exec (fn [& command]
                           (reset! call command)
                           "loaded")]
      (is (= "loaded"
             (clock/verify-application-clock! "^/usr/local/bin/app"))))
    (is (= :bash (first @call)))
    (is (= "^/usr/local/bin/app" (nth @call 4)))
    (is (= clock/library (nth @call 5)))
    (is (= clock/control-file (nth @call 6)))))
