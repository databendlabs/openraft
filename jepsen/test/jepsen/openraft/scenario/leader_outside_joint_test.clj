(ns jepsen.openraft.scenario.leader-outside-joint-test
  (:require [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.client :as http]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.scenario.leader-outside-joint :as scenario]))

(def test-config
  {:nodes ["n1" "n2" "n3"]
   :api-port 21001})

(defn- step [f value]
  {:process :nemesis :type :info :f f :value value})

(def ^:private covered
  [(step :prepare-joint {:status :prepared
                         :committed-voter-configs [#{"n1" "n2" "n3"}
                                                   #{"n1" "n3"}]})
   (step :transfer-to-old-half {:status :transferred :leader "n2"})])

(def ^:private recovered
  (step :check-leader-outside-joint-recovery
        {:status :recovered
         :voters #{"n3"}
         :fresh "after-change"
         :sentinel "before-change"}))

(deftest accepts-a-completed-change-that-recovers
  (let [result (scenario/verdict
                test-config
                (conj covered
                      (step :change-from-old-half {:status :changed})
                      recovered))]
    (is (true? (:valid? result)))))

(deftest rejects-any-other-answer-to-the-change
  (doseq [change [{:status :refused :error-variant :LearnerNotFound}
                  {:status :refused :error-variant :InProgress}
                  {:status :refused :error-variant :UncommittedLeaderLog}
                  {:status :not-leader}
                  nil]]
    (testing (pr-str change)
      (let [result (scenario/verdict
                    test-config
                    (conj covered
                          (step :change-from-old-half change)
                          recovered))]
        (is (false? (:valid? result)))
        (is (false? (:changed? result)))))))

(deftest rejects-an-unanswered-change-or-a-stuck-cluster
  (testing "a change the leader never answers, as when its core panics"
    (let [result (scenario/verdict
                  test-config
                  (conj covered
                        (step :change-from-old-half
                              {:status :indeterminate :kind :transport-error})
                        recovered))]
      (is (false? (:valid? result)))
      (is (true? (:scenario-covered? result)))))

  (testing "a completed change after which the cluster never recovers"
    (is (false? (:valid? (scenario/verdict
                          test-config
                          (conj covered
                                (step :change-from-old-half
                                      {:status :changed})
                                (step :check-leader-outside-joint-recovery
                                      {:status :timeout})))))))

  (testing "a transfer that never reached the old half is not coverage"
    (let [result (scenario/verdict
                  test-config
                  [(first covered)
                   (step :transfer-to-old-half {:status :transferred
                                                :leader "n1"})
                   (step :change-from-old-half {:status :changed})
                   recovered])]
      (is (false? (:scenario-covered? result))))))

(deftest rejects-the-wrong-membership-or-lost-data
  (doseq [recovery [(assoc-in recovered [:value :voters] #{"n1" "n2" "n3"})
                    (assoc-in recovered [:value :fresh] nil)
                    (assoc-in recovered [:value :sentinel] nil)]]
    (let [result (scenario/verdict
                  test-config
                  (conj covered
                        (step :change-from-old-half {:status :changed})
                        recovery))]
      (is (false? (:valid? result)))
      (is (false? (:recovered? result))))))

(deftest classifies-the-change-response
  (testing "a completed change"
    (with-redefs [http/change-membership! (fn [& _] {})]
      (let [result (#'scenario/change! test-config)]
        (is (= {:status :changed :attempts 1} result)))))

  (doseq [[label thrown expected]
          [["a refusal"
            (ex-info "refused" {:kind :openraft-error
                                :error {:ChangeMembershipError
                                        {:LearnerNotFound {}}}})
            {:status :refused :error-variant :LearnerNotFound :attempts 1}]
           ["a lost leadership"
            (ex-info "forward" {:kind :openraft-error
                                :error {:ForwardToLeader {:leader_id "n1"}}})
            {:status :not-leader :attempts 1}]
           ["no answer"
            (ex-info "reset" {:kind :transport-error})
            {:status :indeterminate :kind :transport-error :attempts 1}]]]
    (testing label
      (with-redefs [http/change-membership! (fn [endpoint node-ids]
                                              (is (= "n2:21001" endpoint))
                                              (is (= ["n3"] node-ids))
                                              (throw thrown))]
        (is (= expected (#'scenario/change! test-config))))))

  (testing "a refusal that only means X is not ready yet is retried"
    (let [answers (atom [:InProgress :UncommittedLeaderLog nil])]
      (with-redefs [http/change-membership!
                    (fn [& _]
                      (let [variant (first @answers)]
                        (swap! answers rest)
                        (when variant
                          (throw (ex-info "refused"
                                          {:kind :openraft-error
                                           :error {:ChangeMembershipError
                                                   {variant {}}}})))))]
        (let [result (#'scenario/change! test-config)]
          (is (= {:status :changed :attempts 3} result))))))

  (testing "an unknown error is a Harness failure"
    (with-redefs [http/change-membership! (fn [& _]
                                            (throw (IllegalStateException. "bug")))]
      (is (thrown? IllegalStateException (#'scenario/change! test-config))))))

(deftest recovery-waits-for-the-requested-membership
  (let [bridge {:leader "n3"
                :stable? false
                :effective-voter-configs [#{"n1" "n3"} #{"n3"}]}
        wrong-membership {:leader "n1"
                          :stable? true
                          :voters #{"n1" "n2" "n3"}}
        statuses (atom [bridge wrong-membership (assoc bridge
                                                       :stable? true
                                                       :voters #{"n3"}
                                                       :effective-voter-configs [#{"n3"}])])
        writes (atom [])]
    (with-redefs [cluster/membership-status
                  (fn [_test]
                    (let [status (first @statuses)]
                      (when (next @statuses)
                        (swap! statuses rest))
                      status))
                  http/change-membership!
                  (fn [& _]
                    (throw (IllegalStateException. "recovery must only observe membership")))
                  http/write! (fn [& args]
                                (swap! writes conj (vec args))
                                {:value "after-change" :version 9})
                  http/linearizable-read! (fn [_ k]
                                            {:value (if (= "leader-outside-joint-fresh" k)
                                                      "after-change"
                                                      "before-change")
                                             :version 9})]
      (let [result (#'scenario/recover! test-config)]
        (is (= :recovered (:status result)))
        (is (= #{"n3"} (:voters result)))
        (is (= "after-change" (:fresh result)))
        (is (= "before-change" (:sentinel result)))
        (is (= [["n3:21001" "leader-outside-joint-fresh" "after-change"]]
               @writes))))))
