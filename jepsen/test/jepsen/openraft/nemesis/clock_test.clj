(ns jepsen.openraft.nemesis.clock-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [control :as c]
             [nemesis :as nemesis]]
            [jepsen.openraft.clock :as clock]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.clock :as clock-nemesis]))

(def test-config {:nodes ["n1" "n2" "n3" "n4" "n5"]})

(deftest bump-installs-node-local-offsets
  (let [settings (atom {})
        subject (clock-nemesis/clock-nemesis)]
    (with-redefs [c/on-nodes (fn [_test nodes f]
                               (into {} (map (fn [node] [node (f test-config node)])) nodes))
                  clock/verify-application-clock! (constantly "loaded")
                  clock/write-setting! (fn [setting]
                                         (swap! settings assoc :current setting)
                                         setting)
                  clock-nemesis/verify-offset!
                  (fn [offset-ms]
                    {:setting (get @settings :current)
                     :observed-offset-ms offset-ms})
                  cluster/membership-status (constantly {:leader "n1"})]
      (let [subject (nemesis/setup! subject test-config)
            result (nemesis/invoke! subject
                                    test-config
                                    {:type :info
                                     :f :bump-clock
                                     :value {"n1" 1000 "n2" -2000}})]
        (is (= :installed (get-in result [:value :status])))
        (is (= :bump (get-in result [:value :mode])))
        (is (= true (get-in result [:value :leader-included])))
        (is (= #{"n1" "n2"} (set (get-in result [:value :targets]))))))))

(deftest clock-checker-requires-all-faults-and-final-reset
  (let [subject (:checker (clock-nemesis/clock-package))
        installed (fn [f] {:type :info :f f :value {:status :installed}})]
    (testing "complete coverage"
      (is (true? (:valid? (checker/check subject
                                         test-config
                                         [(installed :bump-clock)
                                          (installed :strobe-clock)
                                          {:type :info
                                           :f :rate-clock
                                           :value {:status :installed
                                                   :direction :fast}}
                                          {:type :info
                                           :f :rate-clock
                                           :value {:status :installed
                                                   :direction :slow}}
                                          (installed :reset-clock)]
                                         {})))))
    (testing "missing bump"
      (is (false? (:valid? (checker/check subject
                                          test-config
                                          [(installed :reset-clock)]
                                          {})))))))

(deftest rate-settings-cover-both-directions
  (dotimes [_ 100]
    (let [fast (#'clock-nemesis/random-rate :fast)
          slow (#'clock-nemesis/random-rate :slow)]
      (is (<= 1.0 fast 2.0))
      (is (<= 0.5 slow 1.0)))))

(deftest strobe-parameters-stay-in-approved-ranges
  (dotimes [_ 100]
    (let [delta-ms (#'clock-nemesis/random-strobe-delta-ms)
          period-ms (#'clock-nemesis/random-strobe-period-ms)
          duration-ms (#'clock-nemesis/random-strobe-duration-ms)]
      (is (<= 4 delta-ms 262144))
      (is (<= 64 period-ms 1024))
      (is (<= 0 duration-ms 32000)))))
