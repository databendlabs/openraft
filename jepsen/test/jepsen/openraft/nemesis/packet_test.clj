(ns jepsen.openraft.nemesis.packet-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft [cluster :as cluster]
             [harness :as harness]
             [worker :as worker]]
            [jepsen.openraft.nemesis.packet :as packet]
            [jepsen.openraft.quorum :as quorum]))

(def nodes ["n1" "n2" "n3" "n4" "n5"])

(defn- recording-nemesis [invocations teardown]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! invocations conj op)
      op)
    (teardown! [_ _test]
      (teardown))))

(defn- installed [details]
  (assoc details :status :installed))

(deftest selects-quorum-safe-leader-aware-targets
  (let [stable [(set nodes)]
        joint [#{"n1" "n2" "n3"}
               #{"n3" "n4" "n5"}]]
    (doseq [configs [stable joint]
            target-role [:leader-included :leader-excluded]]
      (testing (str configs " " target-role)
        (let [targets (#'packet/packet-targets configs
                                               "n1"
                                               target-role)
              survivors (remove targets (quorum/voter-set configs))]
          (is (seq targets))
          (is (quorum/quorum? configs survivors))
          (is (= (= :leader-included target-role)
                 (contains? targets "n1"))))))))

(deftest starts-packet-degradation-with-structured-evidence
  (let [invocations (atom [])
        delegate (recording-nemesis invocations (constantly nil))
        subject (packet/->PacketNemesis delegate :slow)
        op {:type :info
            :process :nemesis
            :f :start-packet
            :value :leader-included}]
    (with-redefs [cluster/membership-status
                  (constantly {:leader "n1"})
                  cluster/voter-configs
                  (constantly [(set nodes)])
                  packet/packet-targets
                  (constantly #{"n1" "n2"})]
      (let [result (nemesis/invoke! subject {:nodes nodes} op)
            delegated (first @invocations)
            value (:value result)]
        (is (= :installed (:status value)))
        (is (= :slow (:mode value)))
        (is (= :leader-included (:target-role value)))
        (is (= "n1" (:leader value)))
        (is (= ["n1" "n2"] (:targets value)))
        (is (= {:delay {:time :300ms
                        :jitter :50ms
                        :correlation :25%
                        :distribution :normal}}
               (:behavior value)))
        (is (= :start-packet (:f delegated)))
        (is (= [["n1" "n2"] (:behavior value)]
               (:value delegated)))))))

(deftest uses-jepsen-default-loss-options-for-flaky-mode
  (let [invocations (atom [])
        subject (packet/->PacketNemesis
                 (recording-nemesis invocations (constantly nil))
                 :flaky)]
    (with-redefs [cluster/membership-status
                  (constantly {:leader "n1"})
                  cluster/voter-configs
                  (constantly [(set nodes)])
                  packet/packet-targets
                  (constantly #{"n2"})]
      (let [result (nemesis/invoke!
                    subject
                    {:nodes nodes}
                    {:type :info
                     :process :nemesis
                     :f :start-packet
                     :value :leader-excluded})]
        (is (= {:loss {}} (get-in result [:value :behavior])))
        (is (= [["n2"] {:loss {}}]
               (:value (first @invocations))))))))

(deftest skips-packet-degradation-without-a-safe-target
  (let [invocations (atom [])
        subject (packet/->PacketNemesis
                 (recording-nemesis invocations (constantly nil))
                 :slow)
        op {:type :info
            :process :nemesis
            :f :start-packet
            :value :leader-excluded}]
    (testing "no supported leader"
      (with-redefs [cluster/membership-status (constantly nil)]
        (let [value (:value (nemesis/invoke! subject {:nodes nodes} op))]
          (is (= :skipped (:status value)))
          (is (= :no-supported-leader (:reason value)))
          (is (empty? @invocations)))))

    (testing "no target for the requested role"
      (with-redefs [cluster/membership-status
                    (constantly {:leader "n1"})
                    cluster/voter-configs
                    (constantly [#{"n1"}])]
        (let [value (:value (nemesis/invoke!
                             subject
                             {:nodes ["n1"]}
                             op))]
          (is (= :skipped (:status value)))
          (is (= :no-safe-packet-target (:reason value)))
          (is (empty? @invocations)))))))

(deftest stop-and-teardown-clear-packet-shaping
  (let [invocations (atom [])
        teardown-count (atom 0)
        subject (packet/->PacketNemesis
                 (recording-nemesis invocations
                                    #(swap! teardown-count inc))
                 :slow)
        result (nemesis/invoke! subject
                                {:nodes nodes}
                                {:type :info
                                 :process :nemesis
                                 :f :stop-packet})]
    (is (= (installed {:mode :slow}) (:value result)))
    (is (= nil (:value (first @invocations))))
    (nemesis/teardown! subject {:nodes nodes})
    (is (= 1 @teardown-count))))

(deftest packet-cleanup-failure-records-a-harness-failure
  (let [failure-state (harness/failure-state)
        error (ex-info "tc failed" {:kind :control-error})
        subject (worker/wrap-nemesis-teardown
                 failure-state
                 :packet
                 (packet/->PacketNemesis
                  (recording-nemesis (atom []) #(throw error))
                  :slow))]
    (nemesis/teardown! subject {:nodes nodes})
    (let [failure (harness/primary-failure failure-state)]
      (is (= :nemesis (:source failure)))
      (is (= :packet (get-in failure [:context :component])))
      (is (identical? error (:throwable failure))))))

(deftest packet-checker-requires-both-target-roles-and-recovery
  (let [subject (#'packet/coverage-checker :slow)
        start (fn [target-role]
                {:type :info
                 :process :nemesis
                 :f :start-packet
                 :value (installed {:mode :slow
                                    :target-role target-role})})
        stop {:type :info
              :process :nemesis
              :f :stop-packet
              :value (installed {:mode :slow})}
        recovered {:type :info
                   :process :nemesis
                   :f :await-recovery
                   :value (installed {:leader "n1"})}]
    (testing "one target role is insufficient"
      (let [result (checker/check subject
                                  {}
                                  [(start :leader-included)
                                   stop
                                   recovered]
                                  {})]
        (is (false? (:valid? result)))
        (is (= [:leader-excluded] (:missing-target-roles result)))
        (is (= :intact (:cluster-state result)))))

    (testing "both target roles and final recovery pass"
      (let [result (checker/check subject
                                  {}
                                  [(start :leader-included)
                                   stop
                                   (start :leader-excluded)
                                   stop
                                   recovered]
                                  {})]
        (is (true? (:valid? result)))
        (is (empty? (:missing-target-roles result)))
        (is (= [:leader-excluded :leader-included]
               (:observed-target-roles result)))))

    (testing "cleanup without confirmed recovery fails"
      (let [result (checker/check subject
                                  {}
                                  [(start :leader-included)
                                   stop
                                   (start :leader-excluded)
                                   stop]
                                  {})]
        (is (false? (:valid? result)))
        (is (= :recovery-pending (:cluster-state result)))))

    (testing "an installed degradation without a target role is a defect"
      (let [result (checker/check subject
                                  {}
                                  [{:type :info
                                    :process :nemesis
                                    :f :start-packet
                                    :value (installed {:mode :slow})}
                                   stop
                                   recovered]
                                  {})]
        (is (= 1 (:unrecognized-installs result)))
        (is (empty? (:observed-target-roles result)))))))

(deftest rejects-unknown-packet-modes
  (is (thrown-with-msg? clojure.lang.ExceptionInfo
                        #"Unknown packet mode"
                        (packet/packet-nemesis nil :drop))))

(deftest chaos-packet-generator-selects-one-mode-per-episode
  (is (= :slow (#'packet/select-packet-mode :slow)))
  (is (every? packet/packet-modes
              (repeatedly 20
                          (fn []
                            (#'packet/select-packet-mode nil))))))

(deftest packet-generator-advances-through-an-episode
  (let [invocations (atom [])]
    (gen-test/simulate
     (gen/limit 4 (#'packet/packet-generator :slow))
     (fn [_context operation]
       (swap! invocations conj operation)
       (assoc operation :type :info)))
    (is (= [[:start-packet :leader-included]
            [:stop-packet nil]
            [:start-packet :leader-excluded]
            [:stop-packet nil]]
           (mapv (fn [op]
                   [(:f op) (get-in op [:value :target-role])])
                 @invocations)))))
