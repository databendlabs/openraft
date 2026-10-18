(ns jepsen.openraft.nemesis.restart-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen [checker :as checker]
             [nemesis :as nemesis]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.restart :as restart]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- status [leader term]
  {:leader leader
   :metrics {leader {:current_term term :last_log_index 40}}})

(defn- control-nemesis [calls respond]
  (reify nemesis/Nemesis
    (setup! [this _test]
      this)
    (invoke! [_ _test op]
      (swap! calls conj [(:f op) (:value op)])
      (assoc op :value (respond op)))
    (teardown! [this _test]
      this)))

(defn- subject [calls respond]
  (restart/->RestartNemesis (control-nemesis calls respond) (atom nil)))

(def ^:private healthy-control
  (fn [{:keys [f value]}]
    {(first value) (case f :kill :killed :start :start-confirmed)}))

(deftest kills-and-restarts-the-leader-at-once
  (doseq [[label after restored?] [["leader restore" (status "n1" 3) true]
                                   ["an election" (status "n2" 4) false]]]
    (testing label
      (let [calls (atom [])
            statuses (atom [(status "n1" 3) after])]
        (with-redefs [cluster/membership-status
                      (fn [_test]
                        (let [s (first @statuses)]
                          (when (next @statuses)
                            (swap! statuses rest))
                          s))]
          (let [value (:value (nemesis/invoke! (subject calls healthy-control)
                                               test-config
                                               {:type :info
                                                :f :crash-restart-leader}))]
            (is (= [[:kill ["n1"]] [:start ["n1"]]] @calls))
            (is (= :installed (:status value)))
            (is (= (:leader after) (:new-leader value)))
            (is (= restored? (:leader-restored? value)))))))))

(deftest leaves-a-leader-another-fault-stopped
  (let [calls (atom [])]
    (with-redefs [cluster/membership-status (constantly (status "n1" 3))]
      (let [value (:value (nemesis/invoke! (subject calls
                                                    (fn [{:keys [value]}]
                                                      {(first value) :target-absent}))
                                           test-config
                                           {:type :info
                                            :f :crash-restart-leader}))]
        (is (= [[:kill ["n1"]]] @calls) "the other fault owns the restart")
        (is (= :target-not-running (:reason value)))))))

(deftest final-cleanup-restarts-a-node-left-down
  (let [calls (atom [])
        nemesis (subject calls
                         (fn [{:keys [f value]}]
                           (if (= :start f)
                             (throw (ex-info "ssh failed" {}))
                             {(first value) :killed})))]
    (with-redefs [cluster/membership-status (constantly (status "n1" 3))]
      (is (thrown? clojure.lang.ExceptionInfo
                   (nemesis/invoke! nemesis test-config
                                    {:type :info :f :crash-restart-leader}))))
    (let [nemesis (assoc nemesis :db-nemesis (control-nemesis calls healthy-control))]
      (is (= {:status :installed :node "n1" :start-result :start-confirmed}
             (:value (nemesis/invoke! nemesis test-config
                                      {:type :info :f :finish-restart})))))))

(deftest requires-a-restart-and-reports-restores
  (let [check #(checker/check (:checker (restart/restart-package :database))
                              test-config
                              %
                              {})
        recovered {:type :info :f :await-recovery
                   :value {:status :installed :leader "n1"}}
        restarted (fn [restored?]
                    {:type :info :f :crash-restart-leader
                     :value {:status :installed :leader "n1"
                             :leader-restored? restored?}})]
    (is (= {:valid? true :restarts 2 :leader-restores 1}
           (select-keys (check [(restarted true) (restarted false) recovered])
                        [:valid? :restarts :leader-restores])))
    (is (false? (:valid? (check [recovered]))) "a run without a restart")
    (is (false? (:valid? (check [(restarted true)]))) "a run without recovery")))
